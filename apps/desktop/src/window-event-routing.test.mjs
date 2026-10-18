// SPDX-License-Identifier: MPL-2.0
// Real bridge and widget subscriptions; synthetic Tauri routing, no native UI or API calls.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule, runInNewContext } from 'node:vm';
import { webcrypto } from 'node:crypto';
import { parse } from '@babel/parser';
import { test } from 'node:test';
import { loadProviderPresets, loadGeometryHistory, loadAssetImport, loadHighlightSource } from './geometry-test-module.mjs';

const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const ts = source => stripTypeScriptTypes(source, { mode: 'transform' });
const plain = value => JSON.parse(JSON.stringify(value));
function bus(failEvent) {
  const subscriptions = new Set();
  let snapshotRevision = 0;
  return {
    subscriptions,
    get snapshotRevision() { return snapshotRevision; },
    async listen(name, callback, options) {
      if (name === failEvent) throw Error('listen failed');
      const entry = { name, callback, target: options?.target };
      subscriptions.add(entry);
      return () => subscriptions.delete(entry);
    },
    // Tauri Any listeners match each targeted emit, including another window.
    emit(target, name, payload) {
      if (name === 'snapshot-revision') snapshotRevision = Math.max(snapshotRevision, payload.revision);
      for (const entry of subscriptions) {
        if (entry.name === name && (entry.target === undefined || entry.target === target)) entry.callback({ payload });
      }
    },
  };
}
async function bridgeFixture(events, native = true) {
  const context = createContext({ crypto: webcrypto, structuredClone, Date, Map, Set, URL, Blob, setTimeout, clearTimeout });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const core = synthetic({ isTauri: () => native, invoke: async command => {
    if (command === 'get_recording_status') return null;
    if (command === 'get_snapshot') return snapshot(events.snapshotRevision);
    assert.equal(command, 'runtime_info');
    return { platform: 'windows', hotkey: 'Alt+A', captureBackend: 'test', contentOrigin: 'http://127.0.0.1:49000' };
  } });
  const event = synthetic({ listen: events.listen });
  const policy = new SourceTextModule(ts(await raw('connection-policy.ts')), { context });
  await policy.link(() => { throw Error('unexpected policy dependency'); });
  const history = await loadGeometryHistory(context), importedAssets = await loadAssetImport(context);
  const mosaic = new SourceTextModule(ts(await raw('mosaic-preview.ts')), { context });
  await mosaic.link(() => { throw Error('unexpected mosaic dependency'); });
  const bridge = new SourceTextModule(ts(await raw('bridge.ts')), { context, initializeImportMeta: meta => { meta.glob = () => ({}); } });
  await bridge.link(async name => name === './highlight-source' ? await loadHighlightSource(context)
    : name === './provider-presets' ? await loadProviderPresets(context)
    : name === './asset-import' ? importedAssets : name === './region-geometry-history' ? history
    : name === './connection-policy' ? policy : name === './mosaic-preview' ? mosaic
    : name.endsWith('/core') ? core : event);
  await bridge.evaluate();
  return bridge.namespace;
}
const snapshot = revision => ({ schemaVersion: 1, revision, scenes: [], agents: [], connection: { baseUrl: '', model: '', hasKey: false } });

test('actual bridge routes each streamed fragment once while preserving intentional repeated text', async () => {
  const events = bus(), bridge = await bridgeFixture(events), text = [], reasoning = [], snapshots = [], errors = [];
  const stop = await bridge.subscribe(value => snapshots.push(value), value => { text.push(value.text ?? ''); reasoning.push(value.reasoning ?? ''); }, value => errors.push(value));
  for (const fragment of ['牛出现在第 ', '4、5 个采样帧', '哈哈', '哈哈']) {
    const payload = { sceneId: 'scene', runId: 'run', text: fragment };
    events.emit('space', 'run-event', payload);
    events.emit('frozen-widget', 'run-event', payload);
  }
  for (const target of ['space', 'frozen-widget']) events.emit(target, 'run-event', { sceneId: 'scene', runId: 'run', reasoning: '思考' });
  events.emit('space', 'snapshot-revision', { revision: 1 }); events.emit('settings', 'snapshot-revision', { revision: 1 });
  events.emit('space', 'host-error', '真实错误'); events.emit('settings', 'host-error', '其他窗口');
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(text.join(''), '牛出现在第 4、5 个采样帧哈哈哈哈');
  assert.equal(reasoning.join(''), '思考');
  assert.equal(snapshots.length, 1); assert.deepEqual(errors, ['真实错误']);
  stop(); assert.equal(events.subscriptions.size, 0);
  events.emit('space', 'run-event', { text: 'late' }); assert.equal(text.join('').includes('late'), false);
});

test('recording status goes once to each surface and late query guards remain active', async () => {
  const events = bus(), bridge = await bridgeFixture(events), space = [], controls = [];
  const stopSpace = await bridge.subscribeRecording(value => space.push(value));
  const stopControls = await bridge.subscribeRecording(value => controls.push(value), 'recording-controls');
  space.length = 0; controls.length = 0;
  const status = { id: 'recording', sceneId: 'scene', phase: 'recording', elapsedMs: 1000, countdown: 0 };
  for (const target of ['space', 'recording-controls']) events.emit(target, 'recording-status', status);
  assert.equal(space.length, 1); assert.equal(controls.length, 1);
  events.emit('space', 'recording-status', { ...status, phase: 'starting', elapsedMs: 0 }); assert.equal(space.length, 1);
  stopSpace(); stopControls(); assert.equal(events.subscriptions.size, 0);
});

async function smallBridge(file, events, initial = null) {
  const context = createContext({ structuredClone, URL, Map, Set });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const module = new SourceTextModule(ts(await raw(file)), { context, initializeImportMeta: meta => { meta.glob = () => ({}); } });
  await module.link(async name => {
    if (name.endsWith('/core')) return synthetic({ isTauri: () => true, invoke: async () => initial });
    if (name.endsWith('/event')) return synthetic({ listen: events.listen });
    if (name === './bridge') return synthetic({ previewDrawingCommand: () => { throw Error('unexpected drawing'); } });
    if (name === './recording-audio') {
      const audio = new SourceTextModule(ts(await raw('recording-audio.ts')), { context });
      await audio.link(() => { throw Error('unexpected audio dependency'); }); return audio;
    }
    throw Error(`unexpected dependency ${name}`);
  });
  await module.evaluate(); return module.namespace;
}

test('scroll controls, plugin settings and audio settings ignore the other window copy', async () => {
  for (const spec of [
    { file: 'scroll-bridge.ts', method: 'subscribeScroll', event: 'scroll-status', target: 'scroll-controls', initial: null, next: { id: 'scroll', sceneId: 'scene', phase: 'capturing', status: 'extended' } },
    { file: 'plugin-bridge.ts', method: 'subscribePlugins', event: 'plugin-state', target: 'settings', initial: null, next: { revision: 2, plugins: [] } },
    { file: 'recording-audio-bridge.ts', method: 'subscribeRecordingAudio', event: 'recording-audio-state', target: 'settings', initial: { revision: 0, sequence: 0, mode: 'mute', grant: null, editable: true }, next: { revision: 1, sequence: 1, mode: 'mute', grant: null, editable: true } },
  ]) {
    const events = bus(), bridge = await smallBridge(spec.file, events, spec.initial), space = [], other = [];
    const stopSpace = await bridge[spec.method](value => space.push(value));
    const stopOther = await bridge[spec.method](value => other.push(value), spec.target);
    space.length = 0; other.length = 0;
    for (const target of ['space', spec.target]) events.emit(target, spec.event, spec.next);
    assert.equal(space.length, 1, spec.event); assert.equal(other.length, 1, spec.event);
    assert.deepEqual(plain(space[0]), spec.next); assert.deepEqual(plain(other[0]), spec.next);
    stopSpace(); stopOther(); assert.equal(events.subscriptions.size, 0);
  }
});

test('settings receives its own snapshot without consuming space streams', async () => {
  const events = bus(), bridge = await bridgeFixture(events), snapshots = [], runs = [], errors = [];
  const stop = await bridge.subscribe(value => snapshots.push(value), value => runs.push(value), value => errors.push(value), 'settings');
  events.emit('space', 'snapshot-revision', { revision: 1 }); events.emit('settings', 'snapshot-revision', { revision: 2 });
  events.emit('space', 'run-event', { text: 'reply' }); events.emit('frozen-widget', 'run-event', { text: 'reply' });
  events.emit('space', 'host-error', 'capture error');
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(snapshots.map(value => value.revision), [2]); assert.equal(runs.length, 0); assert.equal(errors.length, 0);
  const source = await raw('SettingsSurface.tsx');
  const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  const calls = [];
  const visit = node => { if (!node || typeof node !== 'object') return; if (node.type === 'CallExpression' && node.callee.type === 'MemberExpression' && node.callee.object.name === 'bridge' && node.callee.property.name === 'subscribe') calls.push(node); for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value); };
  visit(ast); assert.equal(calls.length, 1); assert.equal(calls[0].arguments[3].value, 'settings');
  stop();
});

test('partial registration failure releases prior listeners; browser preview does not register native events', async () => {
  for (const name of ['run-event', 'host-error']) {
    const events = bus(name), bridge = await bridgeFixture(events);
    await assert.rejects(() => bridge.subscribe(() => {}, () => {}, () => {}), /listen failed/);
    assert.equal(events.subscriptions.size, 0);
  }
  const events = bus(), bridge = await bridgeFixture(events, false);
  const stop = await bridge.subscribe(() => {}, () => {}, () => {}); assert.equal(events.subscriptions.size, 0); stop();
});

test('actual minimized widget registration consumes only its targeted copy of each delta', async () => {
  const events = bus(), source = await raw('FrozenWidget.tsx');
  const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  const component = ast.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration;
  const mount = component.body.body.find(node => node.type === 'ExpressionStatement' && node.expression.type === 'CallExpression' && node.expression.callee.name === 'onMount').expression.arguments[0];
  const reasoning = new SourceTextModule(ts(await raw('reply-reasoning.ts')));
  await reasoning.link(() => { throw Error('unexpected reasoning dependency'); }); await reasoning.evaluate();
  const preview = new SourceTextModule(ts(await raw('frozen-preview.ts')));
  await preview.link(() => new SyntheticModule(Object.keys(reasoning.namespace), function () { for (const key of Object.keys(reasoning.namespace)) this.setExport(key, reasoning.namespace[key]); })); await preview.evaluate();
  const stops = [], snapshots = []; let live = new Map(), expanded = false;
  const mountActual = runInNewContext(ts(`(${source.slice(mount.start, mount.end)})`), {
    isTauri: () => true, listen: events.listen, invoke: async command => { assert.equal(command, 'get_frozen_scenes'); return { revision: 0, scenes: [] }; },
    disposed: false, stops, accept: value => snapshots.push(plain(value)), setExpanded: value => { expanded = value; },
    setLive: update => { live = update(live); }, mergeLivePreview: preview.namespace.mergeLivePreview,
    setError: error => { throw Error(error); },
  });
  await mountActual();
  for (const text of ['牛', '牛']) for (const target of ['space', 'frozen-widget']) events.emit(target, 'run-event', { sceneId: 'scene', runId: 'run', text });
  assert.equal(live.get('scene').text, '牛牛');
  events.emit('space', 'frozen-widget-layout', true); assert.equal(expanded, false);
  events.emit('frozen-widget', 'frozen-widget-layout', true); assert.equal(expanded, true);
  events.emit('frozen-widget', 'frozen-scenes', { revision: 1, scenes: [] }); assert.equal(snapshots.length, 2);
  stops.forEach(stop => stop()); assert.equal(events.subscriptions.size, 0);
});
