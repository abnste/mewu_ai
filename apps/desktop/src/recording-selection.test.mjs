// SPDX-License-Identifier: MPL-2.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const source = await readFile(new URL('./recording-selection.ts', import.meta.url), 'utf8');
const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const ast = parse(app, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const functions = new Map();
let sourceEffect, currentHook;
function visit(node) {
  if (!node || typeof node !== 'object') return;
  if (node.type === 'FunctionDeclaration' && ['startPreparedRecording', 'selectionCompleted'].includes(node.id?.name)) functions.set(node.id.name, app.slice(node.start, node.end));
  if (node.type === 'CallExpression' && node.callee?.name === 'createEffect' && app.slice(node.start, node.end).includes('recordingSelection.reconcile()')) sourceEffect = app.slice(node.start, node.end);
  if (node.type === 'NewExpression' && node.callee?.name === 'RecordingSelection') {
    const property = node.arguments[0].properties.find(value => value.key?.name === 'current');
    currentHook = app.slice(property.value.start, property.value.end);
  }
  for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
}
visit(ast);
const turn = () => new Promise(resolve => setImmediate(resolve));
const owner = { sceneId: 'scene', backgroundId: 'approved-stream' };
function flowFixture(hooks = {}) {
  let serial = 0, valid = true;
  const timers = new Map(), changes = [], starts = [], errors = [];
  const context = vm.createContext({ setTimeout: (callback, delay) => { assert.equal(delay, 120_000); timers.set(++serial, callback); return serial; }, clearTimeout: id => timers.delete(id) });
  const code = stripTypeScriptTypes(source, { mode: 'transform' }).replace('export class RecordingSelection', 'class RecordingSelection');
  vm.runInContext(`${code}\nglobalThis.Flow = RecordingSelection;`, context);
  const flow = new context.Flow({ current: source => valid && (hooks.current?.(source) ?? true), changed: source => changes.push(source ? { ...source } : undefined),
    start: async (...args) => { starts.push(args); await hooks.start?.(...args); }, error: error => errors.push(error.message ?? error) });
  return { flow, timers, changes, starts, errors, set valid(value) { valid = value; } };
}
test('approved-source selection consumes exactly one durable completion without another R or portal', async () => {
  const f = flowFixture(); f.flow.arm(owner);
  assert.equal(f.flow.completed('saved-region'), true);
  assert.equal(f.flow.completed('saved-region'), false);
  await turn(); assert.equal(f.starts.length, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(f.starts)), [[owner, 'saved-region']]);
  assert.equal(f.timers.size, 0); assert.deepEqual(f.changes, [owner, undefined]);
});
test('expired or explicitly canceled selection never invokes a recording start', async () => {
  for (const mode of ['expiry', 'cancel']) {
    const f = flowFixture(); f.flow.arm(owner);
    if (mode === 'expiry') [...f.timers.values()][0](); else f.flow.clear();
    assert.equal(f.flow.completed('saved-region'), false); await turn(); assert.equal(f.starts.length, 0); assert.equal(f.timers.size, 0);
  }
});
test('actual App Solid effect automatically clears a newly armed intent on source, exit and recording changes', () => {
  for (const mode of ['source', 'scene', 'exit', 'recording', 'scroll', 'closed', 'frozen', 'running']) {
    let dispose, arm, move, intent;
    solid.createRoot(cleanup => {
      dispose = cleanup;
      const initial = { id: owner.sceneId, background: { id: owner.backgroundId } };
      const [scene, setScene] = solid.createSignal(initial), [exitPreparing, setExit] = solid.createSignal(false);
      const [recording, setRecording] = solid.createSignal(null), [scroll, setScroll] = solid.createSignal(null);
      const [recordingSelectionSource, setSource] = solid.createSignal();
      const context = vm.createContext({ setTimeout: () => 1, clearTimeout: () => {}, disposed: false,
        scene, exitPreparing, recording, scroll, changed: setSource, createEffect: solid.createEffect, recordingSelectionSource });
      vm.runInContext(`${stripTypeScriptTypes(source, { mode: 'transform' }).replace('export class RecordingSelection', 'class RecordingSelection')}\nconst recordingSelection=new RecordingSelection({current:${currentHook},changed,start:async()=>{},error:error=>{throw error;}});globalThis.flow=recordingSelection;${sourceEffect};`, context);
      arm = () => context.flow.arm(owner); intent = recordingSelectionSource;
      move = () => {
        if (mode === 'source') setScene({ ...initial, background: { id: 'another-source' } });
        if (mode === 'scene') setScene({ ...initial, id: 'another-scene' });
        if (mode === 'exit') setExit(true);
        if (mode === 'recording') setRecording({ phase: 'countdown' });
        if (mode === 'scroll') setScroll({ phase: 'capturing' });
        if (mode === 'closed') setScene({ ...initial, closed: true });
        if (mode === 'frozen') setScene({ ...initial, frozen: true });
        if (mode === 'running') setScene({ ...initial, run: { status: 'running' } });
      };
    });
    arm(); assert.equal(intent()?.backgroundId, owner.backgroundId, mode);
    move(); assert.equal(intent(), undefined, mode);
    dispose();
  }
});
test('source replacement and disposal cancel a completion queued before its microtask', async () => {
  for (const mode of ['source', 'dispose', 'rearm']) {
    const f = flowFixture(); f.flow.arm(owner); f.flow.completed('saved-region');
    if (mode === 'source') { f.valid = false; f.flow.reconcile(); }
    if (mode === 'dispose') f.flow.clear();
    if (mode === 'rearm') f.flow.arm(owner);
    await turn(); assert.equal(f.starts.length, 0, mode); f.flow.clear();
  }
});
test('native lease failure is reported once and does not prevent a later new preparation', async () => {
  let fail = true;
  const f = flowFixture({ start: async () => { if (fail) throw Error('录屏授权已结束，请重新授权'); } });
  f.flow.arm(owner); f.flow.completed('saved-region'); await turn();
  assert.deepEqual(f.errors, ['录屏授权已结束，请重新授权']);
  fail = false; f.flow.arm(owner); f.flow.completed('another-region'); await turn();
  assert.equal(f.starts.length, 2); assert.equal(f.errors.length, 1); f.flow.clear();
});
test('a late failure from a retired owner cannot inject an error into a new source', async () => {
  let reject;
  const f = flowFixture({ start: () => new Promise((_, no) => { reject = no; }) });
  f.flow.arm(owner); f.flow.completed('saved-region'); await turn(); f.valid = false;
  reject(Error('old failure')); await turn(); assert.deepEqual(f.errors, []);
});
function appFixture() {
  let current = { id: 'scene', background: { id: 'approved-stream' }, regions: [{ id: 'saved-region' }] };
  let busy = false, exiting = false, disposed = false, focus = 0, finishes = 0;
  let flush = async () => {}, native = async () => {}, latestAudio = { revision: 1, mode: 'system', grant: { revision: 1 } };
  const selected = latestAudio, grant = { revision: 1 }, starts = [], errors = [], order = [];
  const context = vm.createContext({ scene: () => current, busy: () => busy, recordingReady: () => true, recording: () => null, scroll: () => null,
    exitPreparing: () => exiting, get disposed() { return disposed; }, recordingGrant: () => grant,
    setBusy: value => { busy = value; order.push(['busy', value]); }, spaceOperations: { begin: () => () => { finishes++; } },
    videoPlayers: { pauseAll: () => order.push('pause') }, recordingAudio: { prepare: async () => selected, selection: () => latestAudio },
    flush: async () => { order.push('flush'); await flush(); }, sameAudioGrant: (a, b) => a === b,
    sameAudioSelection: (a, b) => a === b, bridge: { startRecording: async (...args) => { starts.push(args); await native(); } },
    clearReferenceDraft: () => order.push('refs-cleared'), setFocusRequest: f => { focus = f(focus); }, recordingSelection: undefined });
  vm.runInContext(stripTypeScriptTypes(`${functions.get('startPreparedRecording')}\n${functions.get('selectionCompleted')}\nglobalThis.api={startPreparedRecording,selectionCompleted};`, { mode: 'transform' }), context);
  const f = flowFixture({ current: source => !disposed && !exiting && current?.id === source.sceneId && current.background?.id === source.backgroundId,
    start: (...args) => context.api.startPreparedRecording(...args) });
  context.recordingSelection = f.flow;
  return { ...f, api: context.api, starts, order, get busy() { return busy; }, get finishes() { return finishes; }, get focus() { return focus; },
    set scene(value) { current = value; }, set exiting(value) { exiting = value; }, set disposed(value) { disposed = value; },
    set audio(value) { latestAudio = value; }, set flush(value) { flush = value; }, set native(value) { native = value; }, selected, grant };
}
test('actual App completion starts only the saved region of the approved new background', async () => {
  const f = appFixture(); f.flow.arm(owner); f.api.selectionCompleted({ id: 'saved-region' }); await turn();
  assert.deepEqual(JSON.parse(JSON.stringify(f.starts)), [['scene', 'saved-region', f.selected, f.grant]]);
  assert.equal(f.busy, false); assert.equal(f.finishes, 1); assert.equal(f.focus, 0); assert.ok(f.order.includes('refs-cleared'));
  f.api.selectionCompleted({ id: 'ordinary-selection' }); assert.equal(f.focus, 0);
});
test('actual App rejects missing saved regions and source/audio changes during flush', async () => {
  for (const change of ['missing-region', 'source', 'scene', 'exit', 'disposed', 'audio']) {
    const f = appFixture();
    if (change === 'missing-region') f.scene = { id: 'scene', background: { id: 'approved-stream' }, regions: [] };
    f.flush = async () => {
      if (change === 'source') f.scene = { id: 'scene', background: { id: 'different-monitor-stream' }, regions: [{ id: 'saved-region' }] };
      if (change === 'scene') f.scene = { id: 'other', background: { id: 'approved-stream' }, regions: [{ id: 'saved-region' }] };
      if (change === 'exit') f.exiting = true;
      if (change === 'disposed') f.disposed = true;
      if (change === 'audio') f.audio = { ...f.selected, revision: 2 };
    };
    await assert.rejects(f.api.startPreparedRecording(owner, 'saved-region'), change === 'audio' ? /声音设置已变化/ : /录屏选区已变化/);
    assert.equal(f.starts.length, 0, change); assert.equal(f.busy, false, change);
    assert.equal(f.finishes, change === 'missing-region' ? 0 : 1, change);
  }
});
test('actual App finally releases operation ownership after native startup failure', async () => {
  const f = appFixture(); f.native = async () => { throw Error('tray admission failed'); };
  await assert.rejects(f.api.startPreparedRecording(owner, 'saved-region'), /tray admission failed/);
  assert.equal(f.busy, false); assert.equal(f.finishes, 1); assert.equal(f.order.includes('refs-cleared'), false);
});
