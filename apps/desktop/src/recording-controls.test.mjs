// SPDX-License-Identifier: MPL-2.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';

const { nativeSelectOwnsEscape } = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL('./native-select-escape.ts', import.meta.url), 'utf8'), { mode: 'transform' })).toString('base64')}`);

const source = await readFile(new URL('./RecordingSurface.tsx', import.meta.url), 'utf8');
const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const nodes = new Map(); let pauseButton, stopButton, timer;
function visit(node) {
  if (!node || typeof node !== 'object') return;
  if (node.type === 'FunctionDeclaration' && ['act', 'beginDrag'].includes(node.id?.name)) nodes.set(node.id.name, node);
  if (node.type === 'VariableDeclarator' && ['timeLabel', 'canceling', 'canPause', 'stopOrCancel', 'keyboard'].includes(node.id?.name)) nodes.set(node.id.name, node.init);
  if (node.type === 'JSXOpeningElement' && node.name?.name === 'button') {
    const className = node.attributes.find(attribute => attribute.name?.name === 'class')?.value?.value;
    if (className === 'recording-control-button') pauseButton = node;
    if (className === 'recording-control-button recording-stop') stopButton = node;
  }
  if (node.type === 'JSXOpeningElement' && node.name?.name === 'output') timer = node;
  for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
}
visit(ast); assert.ok(pauseButton); assert.ok(stopButton); assert.ok(timer);
const code = node => source.slice(node.start, node.end);
const attribute = (button, name) => button.attributes.find(value => value.name?.name === name).value.expression;
function fixture(phase, nativeLinux = true) {
  let state = { id: 'synthetic-recorder', phase, elapsedMs: 0 }, pending = false, error = '', next = Promise.resolve();
  const calls = [], drags = []; let dragResult = Promise.resolve();
  const context = { nativeSelectOwnsEscape, status: () => state, current: () => state, pending: () => pending, setPending: value => { pending = value; },
    setError: value => { error = value; }, nativeLinux, disposed: false,
    getCurrentWindow: () => ({ startDragging() { drags.push('drag'); return dragResult; } }),
    controlRecording: (id, action) => { calls.push({ id, action }); return next; } };
  vm.createContext(context);
  vm.runInContext(stripTypeScriptTypes(`
    const timeLabel = ${code(nodes.get('timeLabel'))};
    const canceling = ${code(nodes.get('canceling'))}, canPause = ${code(nodes.get('canPause'))};
    ${code(nodes.get('act'))}
    ${code(nodes.get('beginDrag'))}
    const stopOrCancel = ${code(nodes.get('stopOrCancel'))}, keyboard = ${code(nodes.get('keyboard'))};
    globalThis.recorder = { act, stopOrCancel, keyboard, timeLabel,
      drag: ${code(attribute(timer, 'onPointerDown'))},
      pause: ${code(attribute(pauseButton, 'onClick'))},
      pauseDisabled: () => (${code(attribute(pauseButton, 'disabled'))}),
      stopDisabled: () => (${code(attribute(stopButton, 'disabled'))}) };
  `, { mode: 'transform' }), context);
  return { api: context.recorder, calls, drags, set phase(value) { state = { ...state, phase: value }; },
    set next(value) { next = value; }, set dragResult(value) { dragResult = value; },
    set disposed(value) { context.disposed = value; }, set status(value) { state = value; }, get error() { return error; } };
}
const turn = () => new Promise(resolve => setImmediate(resolve));

test('the compact recorder preserves countdown cancellation, pause/resume and final stop actions', async () => {
  for (const phase of ['countdown', 'starting', 'recording', 'paused']) {
    const f = fixture(phase); f.api.stopOrCancel(); await turn();
    assert.deepEqual(f.calls, [{ id: 'synthetic-recorder', action: ['countdown', 'starting'].includes(phase) ? 'cancel' : 'stop' }]);
    assert.equal(f.api.pauseDisabled(), !['recording', 'paused'].includes(phase));
  }
  const f = fixture('recording'); f.api.pause(); await turn(); f.phase = 'paused'; f.api.pause(); await turn();
  assert.deepEqual(f.calls.map(value => value.action), ['pause', 'resume']);
  assert.equal(f.api.timeLabel(125_000), '02:05');
});

test('pending control prevents duplicate dispatch and failure restores usable stop/pause controls', async () => {
  const f = fixture('recording'); let reject; f.next = new Promise((_, no) => { reject = no; });
  const first = f.api.act('pause'); f.api.stopOrCancel();
  assert.equal(f.api.pauseDisabled(), true); assert.equal(f.api.stopDisabled(), true); assert.equal(f.calls.length, 1);
  reject(new Error('synthetic control failure')); await first;
  assert.match(f.error, /synthetic control failure/);
  assert.equal(f.api.pauseDisabled(), false); assert.equal(f.api.stopDisabled(), false);
});

test('Escape cancels during preparation, stops active capture and cannot redispatch during finalization', async () => {
  for (const phase of ['countdown', 'starting', 'recording', 'paused', 'stopping']) {
    const f = fixture(phase); let prevented = 0;
    f.api.keyboard({ key: 'Escape', preventDefault() { prevented++; }, stopPropagation() {} }); await turn();
    assert.equal(f.calls.length, phase === 'stopping' ? 0 : 1);
    assert.equal(prevented, phase === 'stopping' ? 0 : 1);
    if (phase === 'stopping') assert.equal(f.api.stopDisabled(), true);
  }
});

test('Linux timer uses a real left mouse press for native dragging, without dispatching recording controls', async () => {
  const f = fixture('recording'); let prevented = 0;
  const press = { button: 0, pointerType: 'mouse', preventDefault() { prevented++; } };
  f.api.drag(press); await turn();
  assert.equal(prevented, 1); assert.deepEqual(f.drags, ['drag']); assert.deepEqual(f.calls, []);
  for (const event of [{ ...press, button: 2 }, { ...press, pointerType: 'touch' }, { ...press, pointerType: 'pen' }]) f.api.drag(event);
  f.status = null; f.api.drag(press); f.status = { id: 'synthetic-recorder', phase: 'recording' };
  f.disposed = true; f.api.drag(press);
  const otherPlatform = fixture('recording', false); otherPlatform.api.drag(press);
  assert.deepEqual(f.drags, ['drag']); assert.deepEqual(otherPlatform.drags, []); assert.equal(prevented, 1);
  for (const button of [pauseButton, stopButton]) assert.equal(button.attributes.some(value => value.name?.name === 'onPointerDown'), false);
});

test('pending pause does not trap the Linux timer, and native drag rejection leaves stop usable', async () => {
  const f = fixture('recording'); let completePause;
  f.next = new Promise(resolve => { completePause = resolve; });
  const pause = f.api.act('pause');
  f.dragResult = Promise.reject(new Error('synthetic native drag rejection'));
  f.api.drag({ button: 0, pointerType: 'mouse', preventDefault() {} }); await turn();
  assert.equal(f.drags.length, 1); assert.equal(f.calls.length, 1);
  assert.match(f.error, /native drag rejection/);
  completePause(); await pause; f.api.stopOrCancel(); await turn();
  assert.deepEqual(f.calls.map(value => value.action), ['pause', 'stop']);
});

test('native drag authority stays on the first-party recording-controls window only', async () => {
  const capability = JSON.parse(await readFile(new URL('../src-tauri/capabilities/recording-controls.json', import.meta.url), 'utf8'));
  assert.deepEqual(capability.windows, ['recording-controls']);
  assert.equal(capability.permissions.filter(value => value === 'core:window:allow-start-dragging').length, 1);
  assert.equal(capability.remote, undefined);
});
