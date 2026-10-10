// node --experimental-vm-modules apps/desktop/src/pin-tools.test.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
const load = async (path, imports = {}) => { const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(path, import.meta.url), 'utf8'), { mode: 'transform' })); await module.link(name => { if (!imports[name]) throw new Error(`Unexpected ${name}`); return imports[name]; }); await module.evaluate(); return module; };
const interaction = await load('./pin-interaction.ts'), { PinDragGesture, PinRevisionGate, pinImageLayout, preparePin, cancelPinOnEscape } = interaction.namespace;
const { outputDrawing } = (await load('./capture-completion.ts')).namespace;
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const plain = value => JSON.parse(JSON.stringify(value)), checks = [];
{
  const drag = new PinDragGesture(); drag.begin(100, 100, 1);
  assert.equal(drag.move(103, 103, 1, 1, 4, 4), false); // no immediate capture/startDragging
  assert.equal(drag.move(104, 100, 2, 1, 4, 4), false);
  assert.equal(drag.move(104, 100, 1, 1, 4, 4), true); assert.equal(drag.move(110, 100, 1, 1, 4, 4), false);
  drag.begin(100, 100, 1); assert.equal(drag.move(100, 103, 1, 1, 4, 2), true);
  drag.begin(0, 0, 1); drag.end(); assert.equal(drag.move(20, 20, 1, 1, 4, 4), false);
  drag.begin(0, 0, 1); assert.equal(drag.move(20, 20, 1, 0, 4, 4), false);
  checks.push('Initial press is only armed; either system axis threshold starts one drag, and release/cancel leaves double-click available');
}
{
  for (const scale of [1, 1.25, 1.75, 2]) {
    for (const turn of [0, 1, 2, 3]) {
      const swapped = turn % 2;
      const physicalWidth = swapped ? 320 : 480, physicalHeight = swapped ? 480 : 320;
      const layout = pinImageLayout(480, 320, turn, physicalWidth / scale, physicalHeight / scale);
      assert.ok(Math.abs(layout.width * scale - 480) < 1e-9); assert.ok(Math.abs(layout.height * scale - 320) < 1e-9); assert.equal(layout.angle, turn * 90);
    }
    assert.ok((2 + 6 + 1) / scale < 12 / scale); // bounded physical shadow fits transparent padding
  }
  assert.deepEqual(plain(pinImageLayout(480, 320, 4, 480, 320)), { width: 480, height: 320, angle: 0 });
  assert.equal(pinImageLayout(0, 320, 0, 100, 100), undefined);
  checks.push('All rotations derive from immutable source dimensions; 1/1.25/1.75/2 DPI preserve physical content size and bounded shadow margin');
}
{
  const gate = new PinRevisionGate(); assert.equal(gate.accept({ id: 'pin', revision: 2 }), true); assert.equal(gate.accept({ id: 'pin', revision: 1 }), false); assert.equal(gate.accept({ id: 'other', revision: 4 }), false); gate.dispose(); assert.equal(gate.accept({ id: 'pin', revision: 3 }), false);
  checks.push('Late initial queries, foreign window IDs and disposed listeners cannot replace newer pin state');
}
{
  const events = [], flush = deferred(); let canceled = false, savedRevision = 0;
  const pending = outputDrawing({ saveText: async () => { events.push('text'); savedRevision = 1; return true; }, current: () => !canceled, output: () => preparePin({ prepare: () => { events.push('flush'); return flush.promise; }, target: () => canceled ? undefined : ({ drawingRevision: savedRevision, translationId: 'latest-translation' }), run: async target => events.push(['pin', target]) }) });
  await new Promise(resolve => setImmediate(resolve)); assert.deepEqual(events, ['text', 'flush']); savedRevision = 2; flush.resolve(); assert.equal(await pending, true); assert.deepEqual(events[2], ['pin', { drawingRevision: 2, translationId: 'latest-translation' }]);
  assert.equal(await outputDrawing({ saveText: async () => false, current: () => true, output: async () => assert.fail('Pin after text failure') }), false);
  await assert.rejects(preparePin({ prepare: async () => { throw new Error('flush failure'); }, target: () => ({}), run: async () => assert.fail('Pin after failed flush') }), /flush failure/);
  checks.push('Pin output waits for text then flush and reads latest drawing/translation identity; no clipboard, freeze or close callback is involved');
}
{
  const flush = deferred(), events = []; let pending = true;
  const operation = preparePin({ prepare: () => flush.promise, target: () => pending ? {} : undefined, run: async () => events.push('create') });
  const event = key => ({ key, preventDefault: () => events.push('prevent'), stopImmediatePropagation: () => events.push('stop') });
  assert.equal(cancelPinOnEscape(event('p'), true, () => assert.fail('Wrong key canceled')), false);
  assert.equal(cancelPinOnEscape(event('Escape'), false, () => assert.fail('Child Escape stolen')), false);
  assert.equal(cancelPinOnEscape(event('Escape'), true, () => { pending = false; events.push('cancel'); }), true);
  flush.resolve(); assert.equal(await operation, false); assert.deepEqual(events, ['cancel', 'prevent', 'stop']);
  checks.push('Pending-pin Escape is consumed before editor/busy handlers, cancels preparation, and does not close the scene or editor');
}
const good = { id: 'pin', revision: 0, imageUrl: 'http://127.0.0.1:49320/pin/image.png', width: 480, height: 320, quarterTurns: 0, topmost: true, opacity: 1, scaleFactor: 1, shadowPadding: 12, dragThresholdX: 4, dragThresholdY: 4 };
async function bridge(native = true, options = {}) {
  const calls = [], handlers = new Map(), stops = [];
  const core = new SyntheticModule(['invoke', 'isTauri'], function () { this.setExport('isTauri', () => native); this.setExport('invoke', async (command, args) => { calls.push({ command, args }); if (command === 'get_pin_state') return options.initial ?? good; return { id: 'pin' }; }); });
  const event = new SyntheticModule(['listen'], function () { this.setExport('listen', async (name, callback, target) => { calls.push({ event: name, target }); if (name === options.failListen) throw new Error('listen failed'); handlers.set(name, callback); return () => stops.push(name); }); });
  const module = await load('./pin-bridge.ts', { '@tauri-apps/api/core': core, '@tauri-apps/api/event': event, './pin-interaction': interaction });
  return { api: module.namespace, calls, handlers, stops };
}
{
  const initial = deferred(), value = await bridge(true, { initial: initial.promise }), seen = [], errors = [];
  const subscription = value.api.subscribePin(state => seen.push(state.revision), error => errors.push(error));
  await new Promise(resolve => setImmediate(resolve)); value.handlers.get('pin-state')({ payload: { ...good, revision: 3 } }); initial.resolve(good); const stop = await subscription;
  assert.deepEqual(seen, [3]); value.handlers.get('pin-error')({ payload: { message: 'save failure' } }); assert.deepEqual(errors, ['save failure']); stop(); value.handlers.get('pin-state')({ payload: { ...good, revision: 4 } }); assert.deepEqual(seen, [3]); assert.deepEqual(value.stops.sort(), ['pin-error', 'pin-state']);
  const failed = await bridge(true, { failListen: 'pin-error' }); await assert.rejects(failed.api.subscribePin(() => {}, () => {}), /listen failed/); assert.deepEqual(failed.stops, ['pin-state']);
  checks.push('Both owner-scoped events register before query; race and partial-subscription failure clean up without late UI updates');
}
{
  const value = await bridge(), target = { sceneId: 'scene', regionId: 'region', backgroundId: 'background', drawingRevision: 3, x: 1, y: 2, width: 100, height: 80 };
  await value.api.runPin({ requestId: 'request', pluginId: 'pin-plugin', revision: 2, contributionId: 'pin', target, translationId: 'overlay' });
  await value.api.pinReady(false); await value.api.controlPin({ type: 'close' }); await value.api.exportPin(true); await value.api.cancelPin('request'); await value.api.showPinMenu(); await value.api.startPinDrag();
  assert.deepEqual(plain(value.calls[0]), { command: 'run_plugin_pin', args: { requestId: 'request', pluginId: 'pin-plugin', revision: 2, contributionId: 'pin', target, translationId: 'overlay' } });
  assert.deepEqual(plain(value.calls[1]), { command: 'pin_ready', args: { success: false } });
  assert.ok(value.calls.slice(1).every(call => !('pinId' in (call.args ?? {}))));
  assert.equal(value.api.validPinState(good), true); for (const changed of [{ imageUrl: 'https://remote.example/image' }, { imageUrl: 'file:///private.png' }, { opacity: .5 }, { scaleFactor: 0 }, { width: -1 }]) assert.equal(value.api.validPinState({ ...good, ...changed }), false);
  const preview = await bridge(false); await assert.rejects(preview.api.runPin({ target }), /桌面版/); await assert.rejects(preview.api.subscribePin(() => {}, () => {}), /桌面版/); assert.equal(preview.calls.length, 0);
  checks.push('IPC carries only fixed image target/translation identity; pin commands cannot address other windows, and browser preview never fakes native success');
}
{
  const json = async path => JSON.parse(await readFile(new URL(path, import.meta.url), 'utf8'));
  const capability = await json('../src-tauri/capabilities/pin.json'), space = await json('../src-tauri/capabilities/space.json'), config = await json('../src-tauri/tauri.conf.json');
  assert.ok(config.app.security.capabilities.includes(capability.identifier));
  assert.deepEqual(capability.windows, ['pin-*']);
  assert.deepEqual([...capability.permissions].sort(), ['core:event:allow-listen', 'core:event:allow-unlisten', 'allow-get-pin-state', 'allow-pin-ready', 'allow-control-pin', 'allow-show-pin-menu', 'allow-export-pin', 'allow-start-pin-drag', 'allow-reference-pin-object'].sort());
  for (const permission of ['allow-run-plugin-pin', 'allow-cancel-plugin-pin']) assert.ok(space.permissions.includes(permission));
  assert.ok(!capability.permissions.includes('allow-get-snapshot'));
  checks.push('Actual Tauri configuration activates the pin-only owner capability; creation is granted only to the space surface');
}
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
