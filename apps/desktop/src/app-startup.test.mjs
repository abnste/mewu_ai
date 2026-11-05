// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';

const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const app = ast.program.body.find(n => n.type === 'ExportDefaultDeclaration').declaration;
const mounts = app.body.body.filter(n => n.type === 'ExpressionStatement'
  && n.expression.type === 'CallExpression' && n.expression.callee.name === 'onMount')
  .map(n => n.expression.arguments[0]).filter(n => n.async || source.slice(n.start, n.end).includes('syncNativeLanguage'))
  .map(n => source.slice(n.start, n.end));
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };

function fixture(block) {
  const pending = deferred(), stops = [], errors = [], accepted = [], registrations = [];
  const timers = new Map(), windowEvents = new Map(), documentEvents = new Map();
  const events = map => ({
    addEventListener: (name, callback) => map.set(name, callback),
    removeEventListener: name => map.delete(name),
  });
  const stop = name => () => stops.push(name);
  const response = async name => block === name ? pending.promise : stop(name);
  const context = vm.createContext({
    disposed: false, preferences: () => ({ uiLanguage: 'en' }),
    pinObjectsSubscription: undefined, setPinObjects() {},
    subscribePinObjects: async () => block === 'pins' ? pending.promise : { stop: stop('pins') },
    syncNativeLanguage: async () => { if (block === 'language') await pending.promise; },
    stopExitEvents: undefined, stopPinEscape: undefined, stopTranslationEvents: undefined,
    stopPluginEvents: undefined, stopRecordingEvents: undefined, stopScrollEvents: undefined,
    stopEvents: undefined, stopControlEvents: undefined,
    stopExtensionState: undefined, stopExtensionTriggers: undefined, extensionRecords: [],
    setComposerExtensions() {}, triggerSceneId: () => 'own-scene', mergeExtensionRecords: (_, value) => value,
    attachExtensionTriggers: () => { registrations.push('triggers'); return Object.assign(stop('triggers'), {reconcile() {}}); },
    extensions: { extensionNative: true, subscribeExtensions: () => response('extensions'),
      getExtensions: async () => block === 'extensionCatalog' ? pending.promise : [], invokeExtension: async () => {} },
    exitPreparation: { prepare() {}, cancel() {} },
    pinBridge: { subscribePinEscape: () => response('pinEscape') },
    translationBridge: { subscribeTranslation: () => response('translation') },
    translationRequests: { progress() {} },
    plugins: { subscribePlugins: () => response('plugins'), getPlugins: async () => ({ revision: 1, plugins: [] }) },
    acceptPlugins() {}, recordingEvent() {}, scrollEvent() {},
    setRecordingReady() {}, setScrollReady() {},
    scrollBridge: { subscribeScroll: () => response('scroll') },
    showError: e => errors.push(e), runEvent() {}, accept: s => accepted.push(s), setHotkey() {},
    bridge: {
      native: true, subscribeExitPreparation: () => response('exit'),
      subscribeRecording: () => response('recording'), subscribe: () => response('snapshot'),
      getSnapshot: async () => ({ revision: 4, activeSceneId: 'own-scene' }),
      runtimeInfo: async () => ({ hotkey: 'test' }),
      registerControlRenderer: async () => { registrations.push('registered'); return { rendererId: 'renderer' }; },
    },
    attachAppControl: async () => stop('control'), scene: () => undefined,
    snapshot: () => undefined, draft() {}, refs() {}, exitPreparing: () => false,
    flushControl() {}, executeControl() {}, AppControlError: Error,
    document: { dispatchEvent() {}, ...events(documentEvents) }, window: events(windowEvents),
    setTimeout: (fn, ms) => { timers.set(ms, fn); return ms; }, clearTimeout: id => timers.delete(id),
    onCleanup() {}, setStartupError: e => errors.push(e), t: s => s, KeyboardEvent: class {},
  });
  const tasks = mounts.map(code => vm.runInContext(stripTypeScriptTypes(`(${code})()`, { mode: 'transform' }), context));
  return { context, pending, tasks, stops, errors, accepted, registrations, timers, windowEvents };
}

test('a slow listener does not strand the visible space and registration catches up missed publications', async () => {
  const f = fixture('snapshot');
  await tick();
  f.timers.get(250)();
  await tick();
  assert.deepEqual(f.accepted.map(s => s.revision), [4]);
  f.context.bridge.getSnapshot = async () => ({ revision: 5 });
  f.pending.resolve(() => {});
  await Promise.all(f.tasks);
  assert.deepEqual(f.accepted.map(s => s.revision), [4, 5]);
});

test('showing a space whose first read failed obtains its current scene without restarting', async () => {
  const f = fixture('snapshot');
  f.context.bridge.getSnapshot = async () => { throw Error('session unavailable'); };
  f.pending.resolve(() => {});
  await Promise.all(f.tasks);
  assert.equal(f.accepted.length, 0);
  f.context.bridge.getSnapshot = async () => ({ revision: 6, activeSceneId: 'recovered' });
  f.windowEvents.get('focus')();
  await tick();
  assert.equal(f.accepted.at(-1).activeSceneId, 'recovered');
});

test('actual App obtains its initial scene and registers control without waiting for unrelated startup IPC', async () => {
  for (const blocked of ['pins', 'language', 'pinEscape', 'translation', 'plugins', 'recording', 'scroll', 'exit', 'extensions', 'extensionCatalog']) {
    const f = fixture(blocked); await tick();
    assert.equal(f.accepted.length, 1, blocked);
    assert.equal(f.accepted[0].activeSceneId, 'own-scene', blocked);
    assert.equal(f.registrations.filter(value => value === 'registered').length, 1, blocked);
    assert.equal(f.errors.length, 0, blocked);
  }
});

test('a subscription completed after App disposal is stopped without installing late state', async () => {
  for (const blocked of ['pins', 'pinEscape', 'translation', 'plugins', 'recording', 'scroll', 'exit', 'snapshot', 'extensions']) {
    const f = fixture(blocked); await tick(); f.context.disposed = true;
    const stop = () => f.stops.push(blocked);
    f.pending.resolve(blocked === 'pins' ? { stop } : stop); await Promise.all(f.tasks);
    assert.ok(f.stops.includes(blocked), blocked);
    if (blocked === 'snapshot') assert.equal(f.accepted.length, 0);
  }
});

test('late extension catalog after disposal neither attaches trigger keys nor retains its subscription', async () => {
  const f = fixture('extensionCatalog'); await tick(); f.context.disposed = true;
  f.pending.resolve([]); await Promise.all(f.tasks);
  assert.deepEqual(f.registrations, ['registered']);
  assert.equal(f.stops.filter(value => value === 'extensions').length, 1);
  assert.equal(f.context.stopExtensionState, undefined);
});

test('failed extension catalog detaches the already registered state listener', async () => {
  const f = fixture('extensions'); await tick();
  f.context.extensions.getExtensions = async () => { throw Error('offline'); };
  f.pending.resolve(() => f.stops.push('extensions')); await Promise.all(f.tasks);
  assert.equal(f.stops.filter(value => value === 'extensions').length, 1);
  assert.equal(f.context.stopExtensionState, undefined);
  assert.equal(f.errors.length, 1);
  assert.equal(f.registrations.includes('triggers'), false);
});
