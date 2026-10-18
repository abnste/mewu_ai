// Actual App drawing handler, accept fence, PublishedRevision and history lane.
// Synthetic IPC/state only; no product, user files or native input.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';

const plain = value => JSON.parse(JSON.stringify(value));
const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const transform = text => stripTypeScriptTypes(text, { mode: 'transform' });
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const tick = () => new Promise(resolve => setImmediate(resolve));
const source = await raw('./App.tsx');
const tree = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
function unique(kind, name) {
  const matches = [];
  function walk(node) {
    if (!node || typeof node !== 'object') return;
    if (node.type === kind && node.id?.name === name) matches.push(node);
    for (const value of Object.values(node)) {
      if (Array.isArray(value)) value.forEach(walk);
      else if (value && typeof value === 'object' && value.type) walk(value);
    }
  }
  walk(tree); assert.equal(matches.length, 1, `exercise unique production ${name}`);
  return matches[0];
}
const handler = unique('FunctionDeclaration', 'drawingCommand');
const accept = unique('VariableDeclarator', 'accept').init;
const barrier = unique('VariableDeclarator', 'publishedRevision').init;
const pen = id => ({ id, kind: 'pen', color: '#EE4848', strokeWidth: 4, points: [{ x: 1, y: 2 }, { x: 3, y: 4 }] });
const step = (drawing, index) => ({ index, before: null, after: drawing });
const a = pen('a'), b = pen('b'), c = pen('c');
const region = (revision = 2, drawings = [a, b], undo = [step(a, 0), step(b, 1)], redo = []) => ({ id: 'roi', drawingRevision: revision, drawings, drawingHistory: { undo, redo } });
const snapshot = (revision, value = region(), background = 'bg') => ({ revision, activeSceneId: 'scene', scenes: [{ id: 'scene', closed: false, frozen: false, background: { id: background }, regions: [value] }] });
const command = (type = 'undo_drawing', expectedRevision = 2) => ({ type, sceneId: 'scene', regionId: 'roi', backgroundId: 'bg', expectedRevision });

async function fixture() {
  const timers = new Map(); let timerId = 0;
  const context = createContext({ console, structuredClone, setTimeout: fn => { const id = ++timerId; timers.set(id, fn); return id; }, clearTimeout: id => timers.delete(id) });
  async function load(file) {
    const module = new SourceTextModule(transform(await raw(file)), { context });
    await module.link(() => { throw Error(`Unexpected runtime import in ${file}`); });
    await module.evaluate(); return module.namespace;
  }
  const { PublishedRevision } = await load('./published-revision.ts');
  const { DrawingHistoryLane } = await load('./components/drawing-history.ts');
  const state = { snapshot: snapshot(10), exiting: false, scrolling: false, refresh: null };
  const calls = [], accepted = [], refreshes = [], previews = [], completions = [], failures = [];
  const values = { PublishedRevision, snapshot: () => state.snapshot,
    plugins: { applyPluginDrawing: (...args) => { const gate = deferred(); calls.push({ args: plain(args), gate }); return gate.promise; } },
    exitPreparing: () => state.exiting, scroll: () => state.scrolling,
    streamBuffer: { reconcile() {} }, voice: { reconcile() {} }, regionGeometry: { reconcile() {} }, clearSceneState() {},
    setSnapshot: value => { state.snapshot = value; accepted.push(value); },
    bridge: { getSnapshot: async () => { refreshes.push(true); if (state.refresh instanceof Error) throw state.refresh; return state.refresh ?? state.snapshot; } } };
  const imports = new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const owner = new SourceTextModule(transform(`import {${Object.keys(values).join(',')}} from 'fixture';
    let commands=Promise.resolve(),exitFailure,disposed=false,drawingFlushDepth=0;
    const accept=${source.slice(accept.start, accept.end)};
    const publishedRevision=${source.slice(barrier.start, barrier.end)};
    ${source.slice(handler.start, handler.end)}
    export {drawingCommand,accept}; export const drained=()=>commands;
    export const failure=()=>exitFailure; export const flushDepth=value=>{drawingFlushDepth=value;};
    export const dispose=()=>{disposed=true;publishedRevision.dispose();};`), { context });
  await owner.link(() => imports); await owner.evaluate();
  const invoke = value => owner.namespace.drawingCommand('mewu.core.drawing', 1, 'drawing-tools', value);
  const event = value => owner.namespace.accept(plain(value));
  const lane = new DrawingHistoryLane({
    read: () => { const value = state.snapshot.scenes[0].regions[0]; return { source: 'same-frame', revision: value.drawingRevision, drawings: value.drawings, ...value.drawingHistory }; },
    active: () => !state.exiting,
    save: (redo, expectedRevision) => invoke(command(redo ? 'redo_drawing' : 'undo_drawing', expectedRevision)),
    changed: (value, added) => previews.push(plain({ value, added })), completed: (redo, value) => completions.push(plain({ redo, value })), failed: error => failures.push(error),
    add: { save: (drawing, expectedRevision) => invoke({ ...command('add_drawing', expectedRevision), drawing }), remember() {}, forget() {} },
  });
  async function timeout() { const pending = [...timers.values()]; assert.equal(pending.length, 1); await pending[0](); }
  return { state, calls, accepted, refreshes, previews, completions, failures, lane, invoke, event, timeout, ...owner.namespace, close: () => owner.namespace.dispose() };
}

test('publication before reply never completes before the native receipt', async () => {
  const f = await fixture(); try {
    let done = false; const saving = f.invoke(command()).then(() => { done = true; }); await tick();
    f.event(snapshot(11, region(3, [a], [step(a, 0)], [step(b, 1)]))); await tick();
    assert.equal(done, false); f.calls[0].gate.resolve(11); await saving;
    assert.equal(done, true); assert.equal(f.accepted.length, 1); assert.equal(f.refreshes.length, 0);
  } finally { f.close(); }
});

test('reply before publication holds the real command queue and flush drain', async () => {
  const f = await fixture(); try {
    const first = f.invoke(command()), second = f.invoke({ ...command('add_drawing', 3), drawing: c }); await tick();
    assert.equal(f.calls.length, 1); f.calls[0].gate.resolve(11); await tick();
    let drained = false; const flush = f.drained().then(() => { drained = true; }); await tick();
    assert.equal(drained, false); assert.equal(f.calls.length, 1);
    f.event(snapshot(11, region(3, [a], [step(a, 0)], [step(b, 1)]))); await first; await tick();
    assert.equal(f.calls.length, 2); assert.equal(f.calls[1].args[3].expectedRevision, 3);
    f.calls[1].gate.resolve(12); f.event(snapshot(12, region(4, [a, c], [step(a, 0), step(c, 1)]))); await second; await flush;
    assert.equal(drained, true); assert.equal(f.refreshes.length, 0);
  } finally { f.close(); }
});

test('missing event recovers through the production ordinary getSnapshot hook', async () => {
  const f = await fixture(); try {
    const saving = f.invoke(command()); await tick(); f.calls[0].gate.resolve(11); await tick();
    f.state.refresh = snapshot(11, region(3, [a], [step(a, 0)], [step(b, 1)])); await f.timeout(); await saving;
    assert.equal(f.refreshes.length, 1); assert.equal(f.accepted.length, 1); assert.equal(f.state.snapshot.revision, 11);
  } finally { f.close(); }
});

test('late older publication cannot replace a newer accepted drawing source', async () => {
  const f = await fixture(); try {
    const saving = f.invoke(command()); await tick(); f.calls[0].gate.resolve(11); await tick();
    f.event(snapshot(12, region(3, [a], [step(a, 0)], [step(b, 1)])));
    f.event(snapshot(10)); await saving;
    assert.equal(f.state.snapshot.revision, 12); assert.equal(f.accepted.length, 1);
    assert.deepEqual(plain(f.state.snapshot.scenes[0].regions[0].drawings), [a]);
  } finally { f.close(); }
});

test('source mismatch after a higher publication rejects after a failed ordinary refresh', async () => {
  const f = await fixture(); try {
    const saving = f.invoke(command()); const rejected = assert.rejects(saving, /未收到保存结果/); await tick();
    f.calls[0].gate.resolve(11); f.event(snapshot(12, region(3), 'replacement-bg')); await tick();
    await f.timeout(); await rejected; await f.drained(); assert.equal(f.refreshes.length, 1);
  } finally { f.close(); }
});

test('a legal no-op still witnesses its host publication without inventing drawingRevision+1', async () => {
  const f = await fixture(); try {
    const saving = f.invoke({ ...command('update_drawing'), drawing: b }); await tick();
    f.calls[0].gate.resolve(11); f.event(snapshot(11)); await saving;
    assert.equal(f.state.snapshot.scenes[0].regions[0].drawingRevision, 2); assert.equal(f.refreshes.length, 0);
  } finally { f.close(); }
});

test('real history lane queues undo then immediate drawing and preserves exact ACK/history checks', async () => {
  const f = await fixture(); try {
    assert.equal(f.lane.enqueue(false), true);
    assert.equal(f.lane.enqueueAdd(c, f.lane.revisionIdentity()), true); await tick();
    assert.equal(f.calls.length, 1); assert.equal(f.calls[0].args[3].type, 'undo_drawing');
    f.event(snapshot(11, region(3, [a], [step(a, 0)], [step(b, 1)]))); f.calls[0].gate.resolve(11); await tick(); await tick();
    assert.equal(f.calls.length, 2); assert.equal(f.calls[1].args[3].type, 'add_drawing'); assert.equal(f.calls[1].args[3].expectedRevision, 3);
    f.calls[1].gate.resolve(12); await tick(); assert.equal(f.lane.pending(), true);
    f.event(snapshot(12, region(4, [a, c], [step(a, 0), step(c, 1)]))); await f.lane.drain(); await f.drained();
    assert.deepEqual(plain(f.state.snapshot.scenes[0].regions[0]), region(4, [a, c], [step(a, 0), step(c, 1)]));
    assert.equal(f.completions.length, 1); assert.equal(f.failures.length, 0); assert.equal(f.refreshes.length, 0);
  } finally { f.close(); }
});

test('real history lane still rejects a newer event whose drawing contents do not match its own ACK', async () => {
  const f = await fixture(); try {
    assert.equal(f.lane.enqueue(false), true); await tick(); f.calls[0].gate.resolve(11);
    f.event(snapshot(11, region(3, [c], [step(c, 0)], [step(b, 1)])));
    await assert.rejects(f.lane.drain(), /绘制历史回执不一致/); assert.equal(f.failures.length, 1);
  } finally { f.close(); }
});

test('native failure during exit flush remains retained while commands queue recovers', async () => {
  const f = await fixture(); try {
    f.state.exiting = true; f.flushDepth(1); const saving = f.invoke(command()); const rejected = assert.rejects(saving, /durable failure/); await tick();
    f.calls[0].gate.reject(Error('durable failure')); await rejected; await f.drained();
    assert.match(String(f.failure()), /durable failure/); assert.equal(f.accepted.length, 0); assert.equal(f.refreshes.length, 0);
  } finally { f.close(); }
});

test('browser preview retains the original Snapshot return and App acceptance path', async () => {
  const f = await fixture(); try {
    const saving = f.invoke(command()); await tick(); f.calls[0].gate.resolve(snapshot(11)); await saving;
    assert.equal(f.accepted.length, 1); assert.equal(f.refreshes.length, 0);
  } finally { f.close(); }
});

async function bridgeFixture(native) {
  const source = await raw('./plugin-bridge.ts');
  const parsed = parse(source, { sourceType: 'module', plugins: ['typescript'] });
  const nodes = parsed.program.body.filter(node => node.type === 'ExportNamedDeclaration' && node.declaration?.id?.name === 'applyPluginDrawing');
  assert.equal(nodes.length, 1, 'exercise the unique real plugin bridge');
  const calls = [], previewCalls = [], result = snapshot(11);
  const values = { pluginNative: native, invoke: async (...args) => { calls.push(plain(args)); return 11; },
    previewDrawingCommand: async value => { previewCalls.push(plain(value)); return result; },
    desktop: () => { if (!native) throw Error('请在桌面版管理插件'); } };
  const stub = new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
  const module = new SourceTextModule(transform(`import {${Object.keys(values).join(',')}} from 'fixture'; ${source.slice(nodes[0].start, nodes[0].end)}`));
  await module.link(() => stub); await module.evaluate();
  return { apply: module.namespace.applyPluginDrawing, calls, previewCalls, result };
}

test('real native plugin bridge returns only the numeric receipt with unchanged command arguments', async () => {
  const f = await bridgeFixture(true), value = command();
  assert.equal(await f.apply('mewu.core.drawing', 1, 'drawing-tools', value), 11);
  assert.deepEqual(f.calls, [['apply_plugin_drawing', { pluginId: 'mewu.core.drawing', pluginRevision: 1, contributionId: 'drawing-tools', command: value }]]);
  assert.deepEqual(f.previewCalls, []);
});

test('real browser core bridge preserves its original preview result and foreign-plugin rejection', async () => {
  const f = await bridgeFixture(false), value = command();
  assert.equal(await f.apply('mewu.core.drawing', 1, 'drawing-tools', value), f.result);
  assert.deepEqual(f.previewCalls, [value]); assert.deepEqual(f.calls, []);
  await assert.rejects(f.apply('third-party', 1, 'drawing-tools', value), /请在桌面版/);
});
