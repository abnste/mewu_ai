// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, createContext } from 'node:vm';
const source = await readFile(new URL('./drawing-diagnostics.ts', import.meta.url), 'utf8');
const turn = () => new Promise(resolve => setImmediate(resolve));
async function load(globals) {
  const module = new SourceTextModule(stripTypeScriptTypes(source, { mode: 'transform' }), { context: createContext(globals) });
  await module.link(() => { throw Error('Unexpected dependency'); }); await module.evaluate(); return module.namespace;
}
class Target {
  listeners = new Map(); installed = 0; removed = 0;
  addEventListener(name, fn) { this.installed++; this.listeners.set(name, fn); }
  removeEventListener(name, fn) { if (this.listeners.get(name) === fn) { this.removed++; this.listeners.delete(name); } }
  emit(name, target, extra = {}) { this.listeners.get(name)?.({ type: name, target, button: 0, buttons: 1, pointerType: 'mouse', ...extra }); }
}
class Element {
  constructor(category) { this.category = category; }
  closest(selector) {
    const selectors = { color: '.drawing-recent-color', canvas: '.drawing-edit-surface', toolbar: '.drawing-toolbar', editor: '.drawing-editor', button: 'button',
      capture: '.capture-surface', region: '.capture-region', edge: '.capture-handle,.capture-edge' };
    return selectors[this.category] === selector ? this : null;
  }
}
async function fixture() {
  const document = new Target(), window = new Target(), timers = new Map(); let serial = 0, clocks = 0;
  const body = new Element('body'), active = new Element('color');
  Object.assign(document, { body, documentElement: new Element('html'), activeElement: active, hasFocus: () => true });
  const api = await load({ document, window, Element, performance: { now: () => ++clocks },
    setTimeout: (fn, delay) => { assert.equal(delay, 120); timers.set(++serial, fn); return serial; }, clearTimeout: id => timers.delete(id) });
  const tick = async () => { const entries = [...timers.values()]; timers.clear(); for (const fn of entries) fn(); await turn(); };
  return { api, document, window, timers, tick, get clocks() { return clocks; } };
}
test('default and explicitly disabled drawing diagnostics touch no clock DOM observer timer or sender', async () => {
  let reads = 0, sends = 0, stateReads = 0;
  const forbidden = new Proxy({}, { get() { reads++; throw Error('Unexpected diagnostic read'); } });
  const api = await load({ performance: forbidden, document: forbidden, window: forbidden,
    setTimeout: () => { throw Error('Unexpected timer'); }, clearTimeout: () => { throw Error('Unexpected timer'); } });
  const event = new Proxy({}, { get() { throw Error('Must not inspect input in disabled mode'); } });
  api.recordDrawingInput(10, event, () => { stateReads++; throw Error('Must not inspect business state'); });
  api.recordDrawingInput(api.DrawingInputPhase.windowSnapMap, event, undefined, 3);
  api.recordDrawingInput(api.DrawingInputPhase.windowSnapDown, event, undefined, 10);
  api.recordDrawingInput(api.DrawingInputPhase.regionBegin, event, undefined, 6);
  api.recordDrawingInput(api.DrawingInputPhase.regionFinish, event, undefined, 7);
  api.recordDrawingInput(api.DrawingInputPhase.regionEdit, event, undefined, 5);
  assert.equal(api.drawingDiagnosticsEnabled(), false);
  api.startDrawingDiagnostics(false, async () => { sends++; })();
  assert.deepEqual({ reads, sends, stateReads }, { reads: 0, sends: 0, stateReads: 0 });
});

test('snap phases reuse the original fixed numeric row shape and install no extra event observers', async () => {
  const f = await fixture(), sent = [];
  const stop = f.api.startDrawingDiagnostics(true, async payload => sent.push(JSON.parse(payload)));
  assert.equal(f.api.drawingDiagnosticsEnabled(), true);
  f.api.recordDrawingInput(f.api.DrawingInputPhase.windowSnapMap, undefined, undefined, 2);
  f.api.recordDrawingInput(f.api.DrawingInputPhase.windowSnapDown, undefined, undefined, 10);
  await f.tick();
  assert.deepEqual(sent[0].rows.map(row => [row.phase, row.code, row.count]), [[20, 2, 0], [21, 10, 0]]);
  assert.equal(f.document.installed + f.window.installed, 6);
  stop(); assert.equal(f.api.drawingDiagnosticsEnabled(), false);
});

test('capture background region and resize categories use only bounded enums on the existing observers', async () => {
  const f = await fixture(), sent = [];
  const stop = f.api.startDrawingDiagnostics(true, async payload => sent.push(JSON.parse(payload)));
  for (const category of ['capture', 'region', 'edge']) f.document.emit('pointerdown', new Element(category));
  f.api.recordDrawingInput(f.api.DrawingInputPhase.regionBegin, undefined, undefined, 6);
  f.api.recordDrawingInput(f.api.DrawingInputPhase.regionFinish, undefined, undefined, 7);
  f.api.recordDrawingInput(f.api.DrawingInputPhase.regionEdit, undefined, undefined, 5);
  await f.tick();
  assert.deepEqual(sent[0].rows.map(row => [row.phase, row.target, row.code]), [[1, 8, 0], [1, 9, 0], [1, 10, 0], [22, 0, 6], [23, 0, 7], [24, 0, 5]]);
  assert.equal(f.document.installed + f.window.installed, 6);
  stop();
});
test('enabled real observer sends only fixed numeric input categories and lazy guard flags without content', async () => {
  const f = await fixture(), sent = [];
  const stop = f.api.startDrawingDiagnostics(true, async payload => sent.push(JSON.parse(payload)));
  const event = new Proxy({ target: new Element('canvas'), button: 0, buttons: 1, pointerType: 'mouse' }, {
    get(value, name) { if (['clientX', 'clientY', 'textContent', 'id', 'url', 'value'].includes(name)) throw Error('Forbidden content read'); return value[name]; }
  });
  f.api.recordDrawingInput(f.api.DrawingInputPhase.begin, event, () => ({ flags: 5, text: 'never transmitted', id: 'never transmitted' }));
  await f.tick(); assert.equal(sent.length, 1);
  assert.deepEqual(sent[0], { version: 1, rows: [{ sequence: 1, atMicros: 1000, phase: 10, target: 1, active: 3,
    button: 0, buttons: 1, pointer: 1, focused: true, flags: 5, code: 0, count: 0 }] });
  assert.equal(f.document.installed + f.window.installed, 6); assert.equal(f.document.listeners.has('pointermove'), false);
  stop(); assert.equal(f.document.listeners.size + f.window.listeners.size, 0);
});
test('the production queue is singleflight with sixteen-row batches and a total 256-row observer bound', async () => {
  const f = await fixture(), sent = []; let release;
  f.api.startDrawingDiagnostics(true, payload => { sent.push(payload); return sent.length === 1 ? new Promise(resolve => { release = resolve; }) : Promise.resolve(); });
  for (let i = 0; i < 20; i++) f.document.emit('pointerdown', new Element('canvas'));
  await f.tick(); assert.equal(sent.length, 1); assert.equal(JSON.parse(sent[0]).rows.length, 16);
  for (let i = 0; i < 400; i++) f.document.emit('pointerdown', new Element('canvas'));
  assert.equal(sent.length, 1); assert.equal(f.document.listeners.size + f.window.listeners.size, 0);
  release(); await turn(); for (let i = 0; i < 20 && f.timers.size; i++) await f.tick();
  const rows = sent.flatMap(payload => { assert.ok(Buffer.byteLength(payload) <= 8192); return JSON.parse(payload).rows; });
  assert.equal(sent.length, 16); assert.equal(rows.length, 256);
  assert.deepEqual(rows.map(row => row.sequence), Array.from({ length: 256 }, (_, i) => i + 1));
  assert.equal(f.clocks, 257);
});
test('diagnostic state failures and asynchronous native rejection cannot replace a business error or leave observers alive', async () => {
  const f = await fixture(), original = new Error('original business failure'); let sends = 0;
  f.api.startDrawingDiagnostics(true, async () => { sends++; throw Error('diagnostic unavailable'); });
  function business() { f.api.recordDrawingInput(10, undefined, () => { throw Error('advisory state failure'); }); throw original; }
  assert.throws(business, error => error === original);
  f.document.emit('pointerdown', new Element('canvas')); await f.tick();
  assert.equal(sends, 1); assert.equal(f.document.listeners.size + f.window.listeners.size, 0);
  const clocks = f.clocks; f.api.recordDrawingInput(10); assert.equal(f.clocks, clocks);
});
test('disposal ignores a stale native failure and the successor still drains its own input once', async () => {
  const f = await fixture(), sent = []; let reject;
  const oldStop = f.api.startDrawingDiagnostics(true, () => new Promise((_, no) => { reject = no; }));
  f.document.emit('pointerdown', new Element('canvas')); await f.tick(); oldStop();
  const stop = f.api.startDrawingDiagnostics(true, async payload => sent.push(JSON.parse(payload)));
  f.document.emit('pointerdown', new Element('canvas')); reject(Error('retired failure')); await turn(); await f.tick();
  assert.equal(sent.length, 1); assert.equal(sent[0].rows[0].sequence, 2);
  assert.equal(f.document.listeners.size + f.window.listeners.size, 6);
  stop(); await turn(); assert.equal(f.document.listeners.size + f.window.listeners.size, 0);
});
test('invalid advisory numbers are discarded and disposal cancels a queued batch before any IPC', async () => {
  const f = await fixture(); let sends = 0;
  const stop = f.api.startDrawingDiagnostics(true, async () => { sends++; });
  for (const flags of [NaN, Infinity, -1, 4096, 0.5]) f.api.recordDrawingInput(10, undefined, () => ({ flags }));
  assert.equal(f.timers.size, 0);
  f.api.recordDrawingInput(10); stop(); await f.tick(); assert.equal(sends, 0);
});
test('the new diagnostic IPC capability belongs only to the first-party space window', async () => {
  const root = new URL('../src-tauri/capabilities/', import.meta.url); let found = 0;
  for (const name of await readdir(root)) {
    if (!name.endsWith('.json')) continue;
    const capability = JSON.parse(await readFile(new URL(name, root), 'utf8'));
    if (!capability.permissions.includes('allow-record-drawing-diagnostics')) continue;
    found++; assert.deepEqual(capability.windows, ['space']); assert.equal(capability.identifier, 'space');
  }
  assert.equal(found, 1);
});
