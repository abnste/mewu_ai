// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';
import test from 'node:test';

const videoSource = await readFile(new URL('./components/VideoArtifact.tsx', import.meta.url), 'utf8');
const appSource = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
function find(ast, predicate) {
  if (!ast || typeof ast !== 'object') return undefined;
  if (predicate(ast)) return ast;
  for (const value of Object.values(ast)) {
    for (const child of Array.isArray(value) ? value : [value]) {
      const result = find(child, predicate);
      if (result) return result;
    }
  }
}
const video = find(parse(videoSource, { sourceType: 'module', plugins: ['typescript', 'jsx'] }), node => node.type === 'JSXOpeningElement' && node.name.name === 'video');
const keydown = video.attributes.find(node => node.name?.type === 'JSXNamespacedName' && node.name.namespace.name === 'on' && node.name.name.name === 'keydown').value.expression;
const escape = find(parse(appSource, { sourceType: 'module', plugins: ['typescript', 'jsx'] }), node => node.type === 'VariableDeclarator' && node.id.name === 'escape').init;
function fixture(minimized = false) {
  const calls = [];
  const scope = { pausePlayback: () => calls.push('pause'), toggle: () => calls.push('toggle'), openDrawing: () => calls.push('drawing'), nativeSelectOwnsEscape: () => false, exitPreparing: () => false, settings: () => false, pendingDrawingDrafts: () => [], scroll: () => undefined, recording: () => undefined, busy: () => false, sessionsOpen: () => false, scene: () => ({ id: 'owned-video-scene', minimized }), freeze: () => calls.push('freeze'), closeScene: id => calls.push(`close:${id}`) };
  vm.createContext(scope);
  vm.runInContext(stripTypeScriptTypes(`videoKey = (${videoSource.slice(keydown.start, keydown.end)}); appEscape = (${appSource.slice(escape.start, escape.end)});`, { mode: 'transform' }), scope);
  function dispatch(key) {
    const event = { key, isComposing: false, ctrlKey: false, altKey: false, metaKey: false, stopped: false, preventDefault() {}, stopPropagation() { this.stopped = true; } };
    scope.videoKey(event);
    if (!event.stopped) scope.appEscape(event);
    return event;
  }
  return { calls, dispatch, scope };
}
test('actual focused video Escape pauses and reaches ordinary session close', () => {
  const f = fixture();
  assert.equal(f.dispatch('Escape').stopped, false);
  assert.deepEqual(f.calls, ['pause', 'close:owned-video-scene']);
});
test('actual focused video Escape preserves minimized session freeze', () => {
  const f = fixture(true);
  f.dispatch('Escape');
  assert.deepEqual(f.calls, ['pause', 'freeze']);
});
test('actual focused video space and drawing shortcut remain owned by video', () => {
  const f = fixture();
  assert.equal(f.dispatch(' ').stopped, true);
  assert.equal(f.dispatch('d').stopped, true);
  assert.deepEqual(f.calls, ['toggle', 'drawing']);
});

const trimSource = await readFile(new URL('./components/VideoTrimBar.tsx', import.meta.url), 'utf8');
const trimAst = parse(trimSource, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const trimKey = find(trimAst, node => node.type === 'FunctionDeclaration' && node.id.name === 'keyDown');
const trimCancel = find(trimAst, node => node.type === 'FunctionDeclaration' && node.id.name === 'cancelPointer');
const annotationActions = find(parse(videoSource, { sourceType: 'module', plugins: ['typescript', 'jsx'] }), node => node.type === 'JSXOpeningElement' && node.attributes?.some(attribute => attribute.name?.name === 'class' && attribute.value?.value === 'video-preview-annotation-actions'));
const annotationKey = annotationActions.attributes.find(node => node.name?.type === 'JSXNamespacedName' && node.name.namespace.name === 'on' && node.name.name.name === 'keydown').value.expression;
const guardSource = await readFile(new URL('./native-select-escape.ts', import.meta.url), 'utf8');
const guard = parse(guardSource, { sourceType: 'module', plugins: ['typescript'] }).program.body.find(node => node.type === 'ExportNamedDeclaration').declaration;
const { VideoTrimGesture } = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL('./video-trim.ts', import.meta.url), 'utf8'), { mode: 'transform' })).toString('base64')}`);
const trimTurn = () => new Promise(resolve => setImmediate(resolve));
const trimDeferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
class TrimElement {
  constructor(tagName = 'button', editable = false) { this.tagName = tagName; this.editable = editable; this.dataset = {}; }
  closest(selector) { return selector.split(',').some(value => value === this.tagName || value === '[contenteditable="true"]' && this.editable) ? this : null; }
}
function trimEscapeFixture(minimized = false) {
  const f = fixture(minimized), calls = f.calls, errors = [], commits = [], closed = [], exitErrors = [];
  let authority = { identity: 'owned-video', durationTicks: 100_000_000, revision: 3, range: null, editable: true }, position = 20_000_000, view, hold, selected;
  const controller = new VideoTrimGesture({ current: () => authority, position: () => position, playing: () => true,
    pause: () => calls.push('trim-pause'), seek: value => { position = value; }, resume: () => calls.push('trim-resume'), changed: value => { view = value; }, error: value => errors.push(value),
    commit: async edit => { commits.push(edit); if (hold) await hold.promise; authority = { ...authority, revision: authority.revision + 1, range: edit.to }; return { ...authority }; } });
  const root = new TrimElement(), document = { activeElement: root, querySelector: () => document.picker, picker: null };
  Object.assign(f.scope, { controller, Element: TrimElement, document, stopPointer: () => calls.push('release-trim-pointer'),
    disabled: () => Boolean(view?.pending), replay: () => {}, props: { onTogglePlayback() {} },
    selectedAnnotation: () => selected, setSelectedAnnotation: value => { selected = value; calls.push('clear-annotation'); }, video: { focus: () => calls.push('focus-video') } });
  vm.runInContext(stripTypeScriptTypes(`${guardSource.slice(guard.start, guard.end)}\n${trimSource.slice(trimCancel.start, trimCancel.end)}\ntrimKey = (${trimSource.slice(trimKey.start, trimKey.end)}); annotationKey = (${videoSource.slice(annotationKey.start, annotationKey.end)});`, { mode: 'transform' }), f.scope);
  function event(options = {}) { return { key: 'Escape', type: 'keydown', keyCode: 27, target: root, isComposing: false, ctrlKey: false, metaKey: false, altKey: false,
    prevented: false, stopped: false, preventDefault() { this.prevented = true; }, stopPropagation() { this.stopped = true; }, stopImmediatePropagation() { this.stopped = true; }, ...options }; }
  const local = (owner, options) => { const value = event(options); f.scope[owner === 'trim' ? 'trimKey' : 'annotationKey'](value); return value; };
  const dispatch = (owner, options) => { const value = local(owner, options); if (!value.stopped) f.scope.appEscape(value); return value; };
  return { ...f, controller, calls, document, root, errors, commits, closed, exitErrors, local, dispatch,
    get position() { return position; }, get authority() { return authority; }, get view() { return view; },
    select: value => { selected = value; }, selection: () => selected, hold: value => { hold = value; },
    closeThroughFlush() { let flight; f.scope.closeScene = id => { flight = controller.flush(() => true).then(() => closed.push(id), error => exitErrors.push(error)); return flight; }; return () => flight; },
    dispose: () => controller.dispose() };
}

test('idle focused trim and annotation controls let Escape reach the actual App close or minimized freeze without bypassing busy', () => {
  for (const owner of ['trim', 'annotation']) for (const minimized of [false, true]) {
    const f = trimEscapeFixture(minimized);
    try {
      const value = f.dispatch(owner); assert.equal(value.stopped, false); assert.deepEqual(f.calls, [minimized ? 'freeze' : 'close:owned-video-scene']);
      assert.equal(value.prevented, false); assert.equal(f.controller.isInteracting(), false); assert.deepEqual(f.commits, []);
      f.calls.length = 0; f.scope.busy = () => true; f.dispatch(owner); assert.deepEqual(f.calls, [], 'idle controls retain the actual App busy barrier');
    } finally { f.dispose(); }
  }
});

test('only an active trim pointer or keyboard gesture and a selected annotation own the first Escape', () => {
  for (const mode of ['pointer', 'keyboard']) {
    const f = trimEscapeFixture();
    try {
      if (mode === 'pointer') { assert.equal(f.controller.begin('end'), true); f.controller.move(60_000_000); }
      else assert.equal(f.controller.keyDown('end', 'ArrowLeft', false), true);
      assert.equal(f.controller.isInteracting(), true);
      const value = f.dispatch('trim'); assert.equal(value.stopped, true); assert.equal(value.prevented, true); assert.equal(f.controller.isInteracting(), false);
      assert.equal(f.position, 20_000_000); assert.equal(f.calls.filter(value => value === 'trim-resume').length, 1); assert.equal(f.calls.filter(value => value === 'release-trim-pointer').length, 1);
      assert.equal(f.controller.keyUp('ArrowLeft'), undefined); assert.deepEqual(f.commits, []); assert.ok(!f.calls.some(value => value.startsWith('close:')));
      f.calls.length = 0; assert.equal(f.dispatch('trim').stopped, false); assert.deepEqual(f.calls, ['close:owned-video-scene']);
    } finally { f.dispose(); }
  }
  const f = trimEscapeFixture();
  try { f.select('owned-annotation'); assert.equal(f.dispatch('annotation').stopped, true); assert.equal(f.selection(), undefined); assert.deepEqual(f.calls, ['clear-annotation', 'focus-video']);
    f.calls.length = 0; assert.equal(f.dispatch('annotation').stopped, false); assert.deepEqual(f.calls, ['close:owned-video-scene']); }
  finally { f.dispose(); }
});

test('Escape from idle trim during an accepted save reaches close while the real trim flush still waits for success or reports failure', async () => {
  for (const failed of [false, true]) {
    const f = trimEscapeFixture(), native = trimDeferred();
    try {
      f.hold(native); const closing = f.closeThroughFlush(); assert.equal(f.controller.begin('end'), true); f.controller.move(60_000_000);
      const saved = f.controller.complete(); await trimTurn(); assert.equal(f.controller.isInteracting(), false); assert.equal(f.controller.isPending(), true); assert.equal(f.commits.length, 1);
      const value = f.dispatch('trim'); assert.equal(value.stopped, false); assert.ok(closing(), 'the actual App callback must request close'); await trimTurn();
      assert.deepEqual(f.closed, []); assert.deepEqual(f.exitErrors, []); assert.equal(f.controller.isPending(), true); assert.ok(!f.calls.includes('release-trim-pointer'));
      if (failed) native.reject(Error('synthetic native save failure')); else native.resolve();
      assert.equal(await saved, !failed); await closing(); assert.equal(f.controller.isPending(), false);
      assert.equal(f.closed.length, failed ? 0 : 1); assert.equal(f.exitErrors.length, failed ? 1 : 0); assert.equal(f.errors.length, failed ? 1 : 0);
      if (!failed) { assert.equal(f.authority.revision, 4); assert.equal(f.authority.range.endTicks, 60_000_000); }
    } finally { f.dispose(); }
  }
});

test('trim and annotation Escape handlers preserve native select IME and editable-input ownership even with active local content', () => {
  for (const owner of ['trim', 'annotation']) for (const boundary of ['native-picker', 'composing', 'key229', 'input', 'textarea', 'select', 'editable']) {
    const f = trimEscapeFixture();
    try {
      assert.equal(f.controller.begin('end'), true); f.controller.move(60_000_000); f.select('owned-annotation');
      const before = f.calls.length, options = {};
      if (boundary === 'native-picker') f.document.picker = new TrimElement('select');
      if (boundary === 'composing') options.isComposing = true;
      if (boundary === 'key229') options.keyCode = 229;
      if (['input', 'textarea', 'select'].includes(boundary)) options.target = new TrimElement(boundary);
      if (boundary === 'editable') options.target = new TrimElement('span', true);
      const value = f.local(owner, options); assert.equal(value.prevented, false, owner + '/' + boundary); assert.equal(value.stopped, false, owner + '/' + boundary);
      assert.equal(f.calls.length, before); assert.equal(f.controller.isInteracting(), true); assert.equal(f.selection(), 'owned-annotation'); assert.deepEqual(f.commits, []);
    } finally { f.dispose(); }
  }
});
