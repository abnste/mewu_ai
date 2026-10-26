// Run: node --experimental-vm-modules apps/desktop/src/exit-preparation.test.mjs
// Isolated lifecycle/IPC tests. No application, window, native process or credential access.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { webcrypto } from 'node:crypto';
import { parse } from '@babel/parser';
import { loadProviderPresets, loadGeometryHistory, loadAssetImport, loadDrawingDiagnostics, loadHighlightSource } from './geometry-test-module.mjs';

const source = async file => stripTypeScriptTypes(await readFile(new URL(file, import.meta.url), 'utf8'), { mode: 'transform' });
const controlModule = new SourceTextModule(await source('app-control.ts'));
await controlModule.link(() => { throw new Error('Unexpected control dependency'); }); await controlModule.evaluate();
const { AppControlError } = controlModule.namespace;
const module = new SourceTextModule(await source('exit-preparation.ts'));
await module.link(() => { throw new Error('Unexpected runtime dependency'); }); await module.evaluate();
const { pendingSceneDrafts, persistSceneDrafts, SpaceOperations, ExitPreparation } = module.namespace;
const checks = [];
const plain = value => JSON.parse(JSON.stringify(value));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const tick = () => new Promise(resolve => setImmediate(resolve));
const initial = () => ({ activeSceneId: 'a', scenes: [
  { id: 'a', draft: 'old A', refs: [{ kind: 'region', id: 'r1' }] },
  { id: 'b', frozen: true, draft: 'old B', refs: [] },
  { id: 'c', draft: 'untouched', refs: [] },
] });

const state = initial(), drafts = { a: '', b: 'new B' }, refs = { a: [], b: [{ kind: 'item', id: 'i1' }] };
const plan = pendingSceneDrafts(state, drafts, refs);
assert.deepEqual(plain(plan), [
  { type: 'set_draft', sceneId: 'a', draft: '' }, { type: 'set_refs', sceneId: 'a', refs: [] },
  { type: 'set_draft', sceneId: 'b', draft: 'new B' }, { type: 'set_refs', sceneId: 'b', refs: [{ kind: 'item', id: 'i1' }] },
]);
refs.b[0].id = 'changed after planning'; assert.equal(plan[3].refs[0].id, 'i1');
assert.equal(pendingSceneDrafts(state, { a: 'old A' }, { a: [{ kind: 'region', id: 'r1' }] }).length, 0);
assert.throws(() => pendingSceneDrafts(state, { missing: 'keep me' }, {}), /无法找到/);
checks.push('Plans every locally edited scene including frozen scenes and explicit clears; clones refs, skips unchanged/untouched scenes, rejects a missing owner');

let stored = initial(), local = { a: 'first input', b: 'other scene' }, localRefs = { b: [{ kind: 'region', id: 'r2' }] }, writes = [];
await persistSceneDrafts({ active: () => true, snapshot: () => stored, drafts: () => local, references: () => localRefs, failure: () => undefined,
  save: async command => {
    writes.push(plain(command));
    const scene = stored.scenes.find(value => value.id === command.sceneId);
    if (command.type === 'set_draft') scene.draft = command.draft; else scene.refs = plain(command.refs);
    if (writes.length === 1) { await tick(); local = { ...local, a: 'last IME input' }; }
  },
});
assert.equal(stored.scenes[0].draft, 'last IME input'); assert.equal(stored.scenes[1].draft, 'other scene'); assert.deepEqual(stored.scenes[1].refs, [{ kind: 'region', id: 'r2' }]); assert.equal(writes.length, 4);
checks.push('Persistence re-reads local overlays after awaited saves, preserving late IME input while saving another scene and refs');

const unchangedDrafts = { a: 'keep local' }, unchangedRefs = { a: [] };
await assert.rejects(() => persistSceneDrafts({ active: () => true, snapshot: initial, drafts: () => unchangedDrafts, references: () => unchangedRefs, failure: () => undefined, save: async () => { throw new Error('database unavailable'); } }), /database unavailable/);
assert.deepEqual(unchangedDrafts, { a: 'keep local' }); assert.deepEqual(unchangedRefs, { a: [] });
let writeCount = 0;
await assert.rejects(() => persistSceneDrafts({ active: () => true, snapshot: initial, drafts: () => unchangedDrafts, references: () => unchangedRefs, failure: () => new Error('earlier queued drawing failed'), save: async () => { writeCount++; } }), /queued drawing failed/);
assert.equal(writeCount, 0);
checks.push('Save failures preserve draft/ref overlays; a prior queue failure blocks success instead of being hidden by the recovered command chain');

let active = true;
await persistSceneDrafts({ active: () => active, snapshot: initial, drafts: () => ({ a: 'first', b: 'second' }), references: () => ({}), failure: () => undefined, save: async () => { writeCount++; active = false; } });
assert.equal(writeCount, 1);
checks.push('Host cancellation stops remaining queued exit writes');

// Exercise the production flush handoff, not a copy of its comparison logic.
const appSource = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const appAst = parse(appSource, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const appBody = appAst.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration.body.body;
const flushNode = appBody.find(node => node.type === 'FunctionDeclaration' && node.id.name === 'flush');
const flushSource = stripTypeScriptTypes(appSource.slice(flushNode.start, flushNode.end), { mode: 'transform' });
function flushFixture({ queue = Promise.resolve(), state = initial(), text = 'old A', references = [{ kind: 'region', id: 'r1' }], failDrawing = false, save } = {}) {
  const order = [], writes = [], local = { state, text, references, disposed: false, sceneId: 'a' };
  const context = createContext({
    pendingSceneDrafts, AppControlError,
    voice: { cancel: async () => { order.push('voice'); } },
    objectFlush: { flush: async () => { order.push('objects'); } },
    flushVideoDrawing: async () => { order.push('video-drawings'); },
    flushVideo: async () => { order.push('video'); },
    flushDrawing: async () => { order.push('drawings'); if (failDrawing) throw new Error('标注尚未保存'); },
    regionGeometry: { flush: async () => { order.push('geometry'); } },
    scene: () => local.state.scenes.find(scene => scene.id === local.sceneId),
    draft: () => local.text, refs: () => local.references, snapshot: () => local.state,
    clearTimeout: () => { order.push('captured'); },
    get disposed() { return local.disposed; },
    command: async value => { writes.push(plain(value)); if (save) await save(value, local); },
    queue,
  });
  const fixture = new SourceTextModule(`let commands=queue,autosaveVersion=0,debounce;${flushSource};export {flush};export function replaceQueue(value){commands=value;}`, { context });
  return { local, order, writes, async run() { await fixture.link(() => { throw new Error('Unexpected flush dependency'); }); await fixture.evaluate(); return fixture.namespace; } };
}
const idleFlush = flushFixture(), idleApi = await idleFlush.run();
await idleApi.flush();
assert.deepEqual(idleFlush.order, ['voice', 'objects', 'video-drawings', 'video', 'drawings', 'geometry', 'captured']);
assert.deepEqual(idleFlush.writes, []);
const earlierWrite = deferred(), followOnWrite = deferred();
const pendingFlush = flushFixture({ queue: earlierWrite.promise, state: initial(), text: 'queued saved text', references: [] });
const pendingApi = await pendingFlush.run(); let pendingDone = false;
const pendingHandoff = pendingApi.flush().then(() => { pendingDone = true; });
await tick(); assert.equal(pendingDone, false); assert.deepEqual(pendingFlush.writes, []);
pendingFlush.local.state.scenes[0].draft = 'queued saved text'; pendingFlush.local.state.scenes[0].refs = [];
pendingApi.replaceQueue(followOnWrite.promise); earlierWrite.resolve(); await tick(); assert.equal(pendingDone, false);
followOnWrite.resolve(); await pendingHandoff; assert.deepEqual(pendingFlush.writes, []);
checks.push('Actual App flush retains all media/drawing barriers, drains current and follow-on command writes, and skips acknowledged unchanged draft/ref transactions');

const intentWait = deferred(), intentFlush = flushFixture({ queue: intentWait.promise, text: '', references: [] });
const intentApi = await intentFlush.run(), intentHandoff = intentApi.flush(); await tick();
intentFlush.local.text = 'later input'; intentFlush.local.references.push({ kind: 'item', id: 'late-item' });
intentWait.resolve(); await intentHandoff;
assert.deepEqual(intentFlush.writes, [{ type: 'set_draft', sceneId: 'a', draft: '' }, { type: 'set_refs', sceneId: 'a', refs: [] }]);
assert.equal(intentFlush.local.state.scenes[1].draft, 'old B');
checks.push('Actual App flush preserves captured empty-text/reference-clear intent while awaiting IPC, clones refs and touches only its owning scene');

const failedDrawingFlush = flushFixture({ failDrawing: true }), failedDrawingApi = await failedDrawingFlush.run();
await assert.rejects(() => failedDrawingApi.flush(), error => error.message === '截图标注未保存，当前会话已保留' && error.cause.message === '标注尚未保存'); assert.deepEqual(failedDrawingFlush.writes, []);
const changedSceneWait = deferred(), changedSceneFlush = flushFixture({ queue: changedSceneWait.promise, text: 'retain A draft' });
const changedSceneApi = await changedSceneFlush.run(), changedSceneHandoff = changedSceneApi.flush(); await tick();
changedSceneFlush.local.sceneId = 'b'; changedSceneWait.resolve();
await assert.rejects(() => changedSceneHandoff, error => error.cause.message === '会话已变化'); assert.deepEqual(changedSceneFlush.writes, []);
const saveFailureFlush = flushFixture({ text: 'unsaved edit', save: async () => { throw new Error('database unavailable'); } });
const saveFailureApi = await saveFailureFlush.run();
await assert.rejects(() => saveFailureApi.flush(), error => error.message === '会话草稿未保存，当前会话已保留' && error.cause.message === 'database unavailable');
assert.equal(saveFailureFlush.writes.length, 1); assert.equal(saveFailureFlush.local.text, 'unsaved edit');
checks.push('Actual App flush never bypasses unsaved drawing failure, rejects a changed scene after queue drain and preserves local drafts on persistence failure');

const operations = new SpaceOperations(); const finishA = operations.begin(); let drained = false;
const drain = operations.drain(() => true).then(() => { drained = true; }); await tick(); assert.equal(drained, false);
const finishB = operations.begin(); finishA(); await tick(); assert.equal(drained, false); finishB(); await drain; assert.equal(drained, true);
checks.push('Exit waits for already-started operations and follow-on actions before their draft/ref continuations are collected');

const wait = deferred(), locks = [], results = [], errors = []; let flushes = 0;
const preparation = new ExitPreparation({ lock: value => locks.push(value), flush: async () => { flushes++; await wait.promise; }, finish: async result => { results.push(plain(result)); }, error: error => errors.push(error) });
const first = preparation.prepare({ requestId: 'one' }); await preparation.prepare({ requestId: 'one' }); assert.equal(flushes, 1);
await preparation.prepare({ requestId: 'other' }); assert.deepEqual(results, [{ requestId: 'other', success: false, error: '正在准备退出' }]);
preparation.cancel({ requestId: 'obsolete', error: 'ignore' }); assert.deepEqual(locks, [true]); assert.equal(errors.length, 0);
wait.resolve(); await first; assert.deepEqual(results.at(-1), { requestId: 'one', success: true, error: null }); assert.deepEqual(locks, [true]);
preparation.cancel({ requestId: 'one', error: '退出已取消' }); assert.deepEqual(locks, [true, false]); assert.deepEqual(errors, ['退出已取消']); await preparation.prepare({ requestId: 'one' }); assert.equal(flushes, 1);
checks.push('Same nonce is deduplicated; competing/stale requests do not hijack state; successful ack stays locked until host cancel; canceled nonce cannot restart');

const failedResults = [], failedLocks = [];
const failed = new ExitPreparation({ lock: value => failedLocks.push(value), flush: async () => { throw new Error('save failed'); }, finish: async result => { failedResults.push(plain(result)); }, error: () => {} });
await failed.prepare({ requestId: 'fail' }); assert.deepEqual(failedResults, [{ requestId: 'fail', success: false, error: 'save failed' }]); assert.deepEqual(failedLocks, [true, false]);
checks.push('Failure sends one explicit negative acknowledgment and restores editing');

for (const end of ['cancel', 'dispose']) {
  const suspended = deferred(), acknowledgments = [];
  const lifecycle = new ExitPreparation({ lock: () => {}, flush: async () => suspended.promise, finish: async result => { acknowledgments.push(result); }, error: () => { throw new Error('late failure surfaced'); } });
  const running = lifecycle.prepare({ requestId: end });
  if (end === 'cancel') lifecycle.cancel({ requestId: end }); else lifecycle.dispose();
  suspended.reject(new Error('late failure')); await running; assert.equal(acknowledgments.length, 0);
}
checks.push('Canceled/unmounted preparations reject late completions and failures without issuing a stale acknowledgement');

async function bridgeFixture(native, failPrepareListen = false) {
  const events = new Map(), calls = [], stopped = [];
  const context = createContext({ crypto: webcrypto, structuredClone, Date, Map, Set, URL, Blob, setTimeout, clearTimeout });
  const core = new SyntheticModule(['invoke', 'isTauri'], function () { this.setExport('isTauri', () => native); this.setExport('invoke', async (command, args) => { calls.push({ command, args }); }); }, { context });
  const event = new SyntheticModule(['listen'], function () { this.setExport('listen', async (name, callback, options) => { assert.equal(options.target, 'space'); if (failPrepareListen && name === 'prepare-exit') throw new Error('listen failure'); events.set(name, callback); return () => { stopped.push(name); }; }); }, { context });
  const policy = new SourceTextModule(await source('connection-policy.ts'), { context }); await policy.link(() => { throw new Error('unexpected policy import'); });
  const history = await loadGeometryHistory(context); const importedAssets = await loadAssetImport(context);
  const mosaic = new SourceTextModule(await source('mosaic-preview.ts'), { context }); await mosaic.link(() => { throw new Error('unexpected mosaic import'); });
  const bridge = new SourceTextModule(await source('bridge.ts'), { context, initializeImportMeta: meta => { meta.glob = () => ({}); } });
  await bridge.link(async name => name === './drawing-diagnostics' ? await loadDrawingDiagnostics(context) : name === './highlight-source' ? await loadHighlightSource(context) : name === './provider-presets' ? await loadProviderPresets(context) : name === './asset-import' ? importedAssets : name === './region-geometry-history' ? history : name === './connection-policy' ? policy : name === './mosaic-preview' ? mosaic : name.endsWith('/core') ? core : event); await bridge.evaluate();
  return { api: bridge.namespace, events, calls, stopped };
}
const native = await bridgeFixture(true), received = [];
const unlisten = await native.api.subscribeExitPreparation(value => received.push(['prepare', plain(value)]), value => received.push(['cancel', plain(value)]));
assert.deepEqual([...native.events.keys()], ['exit-preparation-canceled', 'prepare-exit']);
native.events.get('prepare-exit')({ payload: { requestId: 'n' } }); native.events.get('prepare-exit')({ payload: { requestId: 5 } }); native.events.get('prepare-exit')({ payload: null });
native.events.get('exit-preparation-canceled')({ payload: { requestId: 'n', error: 'timeout' } });
assert.deepEqual(received, [['prepare', { requestId: 'n' }], ['cancel', { requestId: 'n', error: 'timeout' }]]);
await native.api.finishExitPreparation({ requestId: 'n', success: false, error: 'save error' });
assert.deepEqual(plain(native.calls), [{ command: 'finish_exit_preparation', args: { requestId: 'n', success: false, error: 'save error' } }]);
await native.api.beginExitPreparation({ requestId: 'n' });
assert.deepEqual(plain(native.calls.at(-1)), { command: 'begin_exit_preparation', args: { requestId: 'n' } });
await assert.rejects(() => native.api.beginExitPreparation({ requestId: '' }), /退出/);
assert.equal(native.calls.length, 2);
unlisten(); native.events.get('prepare-exit')({ payload: { requestId: 'late' } }); assert.equal(received.length, 2); assert.deepEqual(native.stopped.sort(), ['exit-preparation-canceled', 'prepare-exit']);
checks.push('Native bridge registers cancel first, scopes both events to space, rejects malformed payloads, sends exact ack and cleans up both listeners');
const partial = await bridgeFixture(true, true); await assert.rejects(() => partial.api.subscribeExitPreparation(() => {}, () => {}), /listen failure/); assert.deepEqual(partial.stopped, ['exit-preparation-canceled']);
const preview = await bridgeFixture(false); const stop = await preview.api.subscribeExitPreparation(() => { throw new Error('preview event'); }, () => {}); stop(); assert.equal(preview.events.size, 0); await assert.rejects(() => preview.api.finishExitPreparation({ requestId: 'x', success: true, error: null }), /桌面应用/); assert.equal(preview.calls.length, 0);
await assert.rejects(() => preview.api.beginExitPreparation({ requestId: 'x' }), /桌面应用/);
assert.equal(preview.calls.length, 0);
checks.push('Partial listener registration cleans up; browser preview does not simulate native exit success');
console.log(JSON.stringify({ passed: true, checks }, null, 2));
