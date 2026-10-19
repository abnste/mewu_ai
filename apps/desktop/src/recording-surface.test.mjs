// SPDX-License-Identifier: MPL-2.0
// Actual recording component and bridge functions; synthetic visible layout/IPC.
// No native windows, screen capture, audio or user configuration writes.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule, createContext } from 'node:vm';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import postcss from 'postcss';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const tick = () => new Promise(resolve => setImmediate(resolve));
const plain = value => JSON.parse(JSON.stringify(value));
const source = await raw('./RecordingSurface.tsx');
const compiled = stripTypeScriptTypes(transformSync(source, { filename: 'RecordingSurface.tsx', configFile: false, babelrc: false,
  parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]] }).code, { mode: 'transform' });
async function fixture({ linux = false, hidden = false, compound = false } = {}) {
  const keys = new Map(), windowKeys = new Map(), calls = [], ready = [], frames = new Map(), timers = new Map();
  let listener, stopped = 0, disposed, serial = 0, visibleSize = true, readyResult = Promise.resolve(true);
  const root = { children: [] };
  const listeners = map => ({ addEventListener(name, fn) { const entries = map.get(name) ?? new Set(); entries.add(fn); map.set(name, entries); }, removeEventListener(name, fn) { const entries = map.get(name); entries?.delete(fn); if (!entries?.size) map.delete(name); } });
  const emit = (map, name, value = {}) => { for (const fn of [...(map.get(name) ?? [])]) fn(value); };
  const document = { hidden, documentElement: { dataset: {} }, ...listeners(keys) };
  const window = { ...listeners(windowKeys) };
  const context = createContext({ document, window, console, Promise, URLSearchParams, location: { search: compound ? '?surface=recording&compound=1' : linux ? '?native-platform=linux' : '' },
    requestAnimationFrame(fn) { const id = ++serial; frames.set(id, fn); return id; }, cancelAnimationFrame(id) { frames.delete(id); },
    setTimeout(fn) { const id = ++serial; timers.set(id, fn); return id; }, clearTimeout(id) { timers.delete(id); },
    localStorage: { getItem: () => JSON.stringify({ maskOpacity: .66 }) } });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const sources = new Map();
  async function loadSource(url) {
    if (sources.has(url.href)) return sources.get(url.href);
    const value = new SourceTextModule(stripTypeScriptTypes(await readFile(url, 'utf8'), { mode: 'transform' }), { context, identifier: url.href });
    sources.set(url.href, value); await value.link(name => loadSource(new URL(`${name}.ts`, url))); return value;
  }
  const preferences = await loadSource(new URL('./interface-preferences.ts', import.meta.url)); await preferences.evaluate();
  const core = synthetic(solid), universal = new SourceTextModule(await raw('../../../node_modules/solid-js/universal/dist/universal.js'), { context });
  await universal.link(() => core); await universal.evaluate();
  const renderer = universal.namespace.createRenderer({
    createElement: type => ({ type, children: [], props: {}, parent: null,
      get isConnected() { let node = this; while (node.parent) node = node.parent; return node === root; },
      getBoundingClientRect: () => ({ width: visibleSize ? 1200 : 0, height: visibleSize ? 800 : 0 }) }),
    createTextNode: text => ({ text, children: [], parent: null }),
    replaceText: (node, text) => { node.text = text; }, setProperty: (node, key, value) => { node.props[key] = value; },
    insertNode: (parent, node, anchor) => { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); const index = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(index < 0 ? parent.children.length : index, 0, node); node.parent = parent; },
    removeNode: (parent, node) => { parent.children.splice(parent.children.indexOf(node), 1); node.parent = null; },
    isTextNode: node => 'text' in node, getParentNode: node => node.parent, getFirstChild: node => node.children[0], getNextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1],
  });
  const module = new SourceTextModule(compiled, { context });
  await module.link(name => {
    if (name === 'solid-js') return core;
    if (name === 'test-renderer') return synthetic(renderer);
    if (name === './i18n') return synthetic({ t: value => value });
    if (name === './interface-preferences') return preferences;
    if (name === './native-select-escape') return synthetic({ nativeSelectOwnsEscape: () => false });
    if (name === './bridge') return synthetic({ subscribeRecording: async (next, surface) => { assert.equal(surface, 'recording-controls'); listener = next; return () => stopped++; }, controlRecording: async (id, action) => { calls.push({ id, action }); }, recordingSurfaceReady: (id, stage) => { ready.push({ id, stage }); return readyResult; } });
    if (name === '@tauri-apps/api/window') return synthetic({ getCurrentWindow: () => ({ startDragging: async () => { calls.push('drag'); } }) });
    if (name === 'lucide-solid') return synthetic(Object.fromEntries(['CircleAlert','LoaderCircle','Pause','Play','Square','X'].map(name => [name, () => null])));
    if (name.endsWith('.css')) return synthetic({});
    throw Error(`Unexpected import ${name}`);
  }); await module.evaluate();
  solid.createRoot(dispose => { disposed = dispose; renderer.insert(root, () => module.namespace.default()); }); await tick();
  const nodes = () => { const result = []; const walk = node => { result.push(node); node.children?.forEach(walk); }; walk(root); return result; };
  const one = key => nodes().find(node => node.props?.class === key);
  const frame = () => { const batch = [...frames.values()]; frames.clear(); batch.forEach(fn => fn()); };
  return { next: value => listener(value), nodes, one, calls, ready, keys, windowKeys, document, frame, frames, timers,
    visible(value) { document.hidden = !value; emit(keys, 'visibilitychange'); },
    size(value) { visibleSize = value; emit(windowKeys, 'resize'); },
    key(key) { emit(keys, 'keydown', { key, preventDefault() {}, stopPropagation() {} }); },
    set readyResult(value) { readyResult = value; },
    dispose: () => disposed(), stopped: () => stopped };
}
const initial = { id: 'record-a', sceneId: 'synthetic', phase: 'starting', countdown: 0, elapsedMs: 0, audio: 'system', stopHotkey: 'F8 / Esc',
  rect: { x: .3, y: .25, width: .4, height: .5 }, hud: { rect: { x: .35, y: .1, width: .2, height: .06 }, stage: 'layout' } };
const maskStatus = () => ({ ...initial, hud: { ...initial.hud, stage: 'mask' } });

test('native compound URL is transparent before any status or HUD arrives and never expands compact controls over the monitor', async () => {
  const f = await fixture({ compound: true });
  assert.equal(f.document.documentElement.dataset.recordingCompound, 'true');
  assert.equal(f.one('recording-control-window').props.classList['recording-compound-window'], true);
  assert.equal(f.one('recording-controls'), undefined); assert.equal(f.one('recording-live-mask'), undefined);
  f.next({ ...initial, hud: undefined }); await tick();
  assert.equal(f.document.documentElement.dataset.recordingCompound, 'true'); assert.equal(f.one('recording-controls'), undefined);
  f.frame(); f.frame(); assert.deepEqual(f.ready, []);
  f.next(initial); await tick(); assert.ok(f.one('recording-controls')); f.frame(); f.frame(); assert.deepEqual(f.ready, [{ id: 'record-a', stage: 'layout' }]);
  f.next(null); await tick(); assert.equal(f.document.documentElement.dataset.recordingCompound, 'true'); assert.equal(f.one('recording-controls'), undefined);
  f.dispose(); assert.equal(f.document.documentElement.dataset.recordingCompound, undefined);
});

test('recording layout waits for two visible layouts, retains one toolbar and leaves countdown on the main surface', async () => {
  const f = await fixture(); f.next(initial); await tick();
  const controls = f.one('recording-controls'), slot = f.one('recording-controls-slot'); assert.ok(controls && slot);
  assert.deepEqual(plain(slot.props.style), { left: '35%', top: '10%', width: '20%', height: '6%' });
  assert.equal(f.one('recording-live-mask'), undefined); assert.deepEqual(f.ready, []);
  f.frame(); assert.deepEqual(f.ready, []); f.frame(); assert.deepEqual(f.ready, [{ id: 'record-a', stage: 'layout' }]);
  f.next({ ...initial, phase: 'countdown', countdown: 3 }); await tick(); f.frame(); f.frame();
  assert.equal(f.one('recording-controls'), controls); assert.equal(f.one('recording-countdown'), undefined); assert.equal(f.ready.length, 1);
  f.dispose(); assert.equal(f.stopped(), 1); assert.equal(f.frames.size, 0); assert.equal(f.timers.size, 0); assert.equal(f.keys.size, 0); assert.equal(f.windowKeys.size, 0);
});

test('mask readiness preserves the exact source hole and configured dimming through live pause and timer updates', async () => {
  const f = await fixture(); f.next(initial); await tick(); f.frame(); f.frame(); const toolbar = f.one('recording-controls');
  f.next(maskStatus()); await tick(); const mask = f.one('recording-live-mask'); assert.ok(mask);
  const cutout = f.nodes().find(node => node.type === 'mask'), hole = cutout.children.find(node => node.props?.fill === 'black');
  assert.deepEqual(plain({ x: hole.props.x, y: hole.props.y, width: hole.props.width, height: hole.props.height }), initial.rect);
  const shade = f.nodes().find(node => node.props?.mask === 'url(#recording-cutout)'); assert.equal(shade.props['fill-opacity'], .66);
  f.frame(); assert.equal(f.ready.length, 1); f.frame(); assert.deepEqual(f.ready.at(-1), { id: 'record-a', stage: 'mask' });
  for (const phase of ['recording', 'paused', 'recording', 'stopping']) { f.next({ ...initial, phase, elapsedMs: 1000, hud: { ...initial.hud, stage: 'live' } }); await tick(); }
  assert.equal(f.one('recording-controls'), toolbar); assert.equal(f.one('recording-live-mask'), mask); assert.equal(f.ready.length, 2); assert.equal(shade.props['fill-opacity'], .66);
  f.next(null); await tick(); assert.equal(f.one('recording-live-mask'), undefined); assert.equal(f.one('recording-controls'), undefined); assert.equal(f.document.documentElement.dataset.recordingCompound, undefined); f.dispose();
});

test('hidden or zero-size recording HUD cannot acknowledge layout, and only its current visible stage can proceed', async () => {
  const f = await fixture({ hidden: true }); f.next(initial); await tick(); f.frame(); f.frame(); assert.deepEqual(f.ready, []);
  f.visible(true); f.frame(); f.visible(false); f.frame(); assert.deepEqual(f.ready, []);
  f.size(false); f.visible(true); f.frame(); f.frame(); assert.deepEqual(f.ready, []);
  f.size(true); f.frame(); f.next(maskStatus()); await tick(); f.frame(); assert.deepEqual(f.ready, []);
  f.frame(); assert.deepEqual(f.ready, [{ id: 'record-a', stage: 'mask' }]); f.dispose();
});

test('ending or replacing a recording revokes pending readiness and late old failures cannot alter its successor', async () => {
  const f = await fixture(); f.next(initial); await tick(); f.frame(); f.next(null); await tick(); f.frame(); assert.deepEqual(f.ready, []);
  let fail; f.readyResult = new Promise((_, reject) => { fail = reject; }); f.next(initial); await tick(); f.frame(); f.frame();
  f.next({ ...initial, id: 'record-b' }); await tick(); fail(new Error('old readiness failure')); await tick();
  assert.equal(f.one('recording-elapsed').props.role, undefined); f.dispose(); f.frame(); assert.equal(f.ready.length, 1); assert.equal(f.frames.size, 0);
});

test('finalizing or expiring a recording cannot send a delayed readiness acknowledgement or retry a rejected one', async () => {
  const f = await fixture(); f.next(initial); await tick(); f.frame();
  f.next({ ...initial, phase: 'stopping' }); await tick(); f.frame(); assert.deepEqual(f.ready, []);
  f.next({ ...initial, id: 'record-b' }); await tick();
  const timeout = [...f.timers.values()]; assert.equal(timeout.length, 1); timeout[0](); f.frame(); f.visible(true); f.frame(); f.frame(); assert.deepEqual(f.ready, []);
  f.next({ ...initial, id: 'record-c' }); await tick(); f.readyResult = Promise.resolve(false); f.frame(); f.frame(); await tick();
  f.size(true); f.visible(true); f.frame(); f.frame(); assert.deepEqual(f.ready, [{ id: 'record-c', stage: 'layout' }]); f.dispose();
});

test('full-monitor recording never invents controls inside the source and keeps explicit cancellation usable', async () => {
  const f = await fixture(); f.next({ ...initial, hud: { stage: 'layout', rect: null } }); await tick();
  assert.equal(f.one('recording-controls-slot'), undefined); f.frame(); f.frame(); assert.deepEqual(f.ready, [{ id: 'record-a', stage: 'layout' }]);
  f.next({ ...maskStatus(), hud: { stage: 'mask', rect: null } }); await tick(); assert.ok(f.one('recording-live-mask')); assert.equal(f.one('recording-controls-slot'), undefined);
  f.key('Escape'); await tick(); assert.deepEqual(f.calls, [{ id: 'record-a', action: 'cancel' }]); f.dispose();
});

test('fallback recording platforms keep compact controls without compound readiness or an extra countdown', async () => {
  const f = await fixture({ linux: true }); f.next({ ...initial, phase: 'countdown', countdown: 3, hud: undefined }); await tick();
  assert.ok(f.one('recording-controls')); assert.equal(f.one('recording-live-mask'), undefined); f.frame(); f.frame(); assert.deepEqual(f.ready, []);
  f.key('Escape'); await tick(); assert.deepEqual(f.calls, [{ id: 'record-a', action: 'cancel' }]); f.dispose(); assert.equal(f.document.documentElement.dataset.recordingPlatform, undefined);
});

test('compound recording CSS stays transparent outside a single native-normalized control slot', async () => {
  const rules = new Map(); postcss.parse(await raw('./recording.css')).walkRules(rule => { const values = rules.get(rule.selector) ?? {}; rule.walkDecls(decl => { values[decl.prop] = decl.value; }); rules.set(rule.selector, values); });
  assert.equal(rules.get('.recording-compound-window')['pointer-events'], 'none');
  assert.equal(rules.get('.recording-compound-window .recording-controls-slot')['pointer-events'], 'auto');
  assert.equal(rules.get('.recording-live-mask')['pointer-events'], 'none');
  assert.equal(rules.get('html[data-recording-compound="true"],html[data-recording-compound="true"] body,html[data-recording-compound="true"] #root').background, 'transparent');
});

const bridgeSource = await raw('./bridge.ts'), bridgeAst = parse(bridgeSource, { sourceType: 'module', plugins: ['typescript'] });
const bridgeFunction = name => { const node = bridgeAst.program.body.map(value => value.declaration).find(value => value?.type === 'FunctionDeclaration' && value.id.name === name); assert.ok(node); return bridgeSource.slice(node.start, node.end); };
async function subscriptionFixture(native = true) {
  const calls = [], values = []; let event;
  const context = createContext({ native, invoke: async (command, args) => { calls.push({ command, args }); return command === 'get_recording_status' ? null : true; }, listen: async (name, fn, options) => { assert.equal(name, 'recording-status'); assert.equal(options.target, 'recording-controls'); event = fn; return () => {}; } });
  const module = new SourceTextModule(stripTypeScriptTypes(`${bridgeFunction('recordingSurfaceReady')}
${bridgeFunction('subscribeRecording')}
export {recordingSurfaceReady,subscribeRecording};`, { mode: 'transform' }), { context });
  await module.link(() => { throw Error('Unexpected bridge import'); }); await module.evaluate();
  const stop = await module.namespace.subscribeRecording(value => values.push(value), 'recording-controls');
  return { calls, values, module, stop, next: value => event({ payload: value }) };
}

test('actual recording subscription admits layout-to-countdown but rejects stale countdown after mask or live', async () => {
  const f = await subscriptionFixture(); f.values.length = 0; f.next(initial); f.next({ ...initial, phase: 'countdown', countdown: 3 }); f.next({ ...initial, phase: 'countdown', countdown: 2 });
  f.next({ ...initial, phase: 'countdown', countdown: 3 }); assert.equal(f.values.length, 3);
  f.next(maskStatus()); f.next({ ...initial, phase: 'countdown', countdown: 1 }); assert.equal(f.values.length, 4);
  f.next({ ...initial, phase: 'recording', hud: { ...initial.hud, stage: 'live' }, elapsedMs: 1000 });
  f.next({ ...initial, phase: 'countdown', countdown: 1 }); f.next(maskStatus()); assert.equal(f.values.length, 5);
  f.next(null); f.next(initial); assert.equal(f.values.at(-1), null); f.stop();
  const fallback = await subscriptionFixture(); fallback.values.length = 0; fallback.next({ ...initial, hud: undefined }); fallback.next({ ...initial, hud: undefined, phase: 'countdown', countdown: 3 }); assert.equal(fallback.values.length, 1); fallback.stop();
});

test('actual recording subscription cannot remove or roll back a ready compound mask on late same-owner status', async () => {
  const f = await subscriptionFixture(); f.values.length = 0; f.next(initial);
  f.next({ ...initial, phase: 'countdown', countdown: 3 });
  f.next(initial); assert.equal(f.values.length, 2);
  f.next(maskStatus());
  f.next(initial); f.next({ ...initial, hud: undefined }); assert.equal(f.values.length, 3);
  const live = { ...initial, hud: { ...initial.hud, stage: 'live' } }; f.next(live);
  f.next(maskStatus()); f.next(initial); f.next({ ...initial, hud: undefined }); assert.equal(f.values.length, 4);
  assert.deepEqual(plain(f.values.at(-1)), live);
  f.next({ ...live, phase: 'recording', elapsedMs: 1000 }); f.next({ ...live, phase: 'stopping', elapsedMs: 1000 });
  assert.equal(f.values.length, 6); f.next(null); assert.equal(f.values.at(-1), null); f.stop();
});

test('actual recording readiness bridge forwards the exact owner/stage and browser review cannot fabricate native readiness', async () => {
  const f = await subscriptionFixture(); assert.equal(await f.module.namespace.recordingSurfaceReady('record-a', 'mask'), true);
  assert.deepEqual(plain(f.calls.at(-1)), { command: 'recording_surface_ready', args: { id: 'record-a', stage: 'mask' } }); f.stop();
  const browser = await subscriptionFixture(false); assert.equal(await browser.module.namespace.recordingSurfaceReady('record-a', 'layout'), false); assert.deepEqual(browser.calls, []); browser.stop();
});
