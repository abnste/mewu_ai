// node --experimental-vm-modules apps/desktop/src/region-geometry.test.mjs
// Geometry/state/IPC fixtures only; no user files, native window or clipboard.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { webcrypto } from 'node:crypto';
import { loadProviderPresets, loadGeometryHistory, loadAssetImport } from './geometry-test-module.mjs';
const source = async file => stripTypeScriptTypes(await readFile(new URL(file, import.meta.url), 'utf8'), { mode: 'transform' });
const mod = new SourceTextModule(await source('./region-geometry.ts')); await mod.link(() => { throw Error('Unexpected runtime import'); }); await mod.evaluate();
const { RegionGeometryCoordinator, geometryTarget, geometryReceipt, overlayRegion, nudgedGeometry } = mod.namespace;
const plain = value => JSON.parse(JSON.stringify(value));
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const checks = [];
const until = async predicate => { for (let i = 0; i < 100 && !predicate(); i++) await new Promise(resolve => setTimeout(resolve, 2)); assert.ok(predicate(), 'fixture did not settle'); };
const makeScene = () => ({ id: 'scene', closed: false, frozen: false, background: { id: 'bg', width: 1000, height: 600 }, regions: [{ id: 'r', x: 100, y: 120, width: 300, height: 200, drawingRevision: 0, drawings: [{ id: 'draw' }], drawingHistory: { undo: [], redo: [] } }] });
function fixture(options = {}) {
  let scene = makeScene(), view, operations = 0;
  const calls = [], waits = [], errors = [], finishes = [];
  const gate = new RegionGeometryCoordinator({ current: (id, regionId) => id === scene.id ? geometryTarget(scene, regionId) : undefined, changed: value => { view = value; }, error: error => errors.push(error.message), delay: 0, newId: () => webcrypto.randomUUID(), beginOperation: () => { operations++; return () => operations--; }, finish: async (sceneId, editId) => { finishes.push({ sceneId, editId }); await options.finish?.(sceneId, editId); }, commit: command => { calls.push(plain(command)); const wait = deferred(); waits.push(wait); return wait.promise; } });
  const finish = (i, external = false) => {
    const command = calls[i], region = scene.regions[0];
    Object.assign(region, command.to, { drawingRevision: command.expectedRevision + 1 });
    const returned = { scenes: [structuredClone(scene)] }, receipt = geometryReceipt(returned, command);
    if (external) { region.x++; region.drawingRevision++; }
    gate.reconcile(); waits[i].resolve(receipt);
  };
  return { gate, calls, waits, errors, finishes, finish, get scene() { return scene; }, set scene(value) { scene = value; gate.reconcile(); }, get view() { return view; }, get operations() { return operations; }, target: () => geometryTarget(scene, 'r') };
}
{
  let value = { x: 699, y: 399, width: 300, height: 200 };
  value = nudgedGeometry(value, 10, 10, 1000, 600); assert.deepEqual(plain(value), { x: 700, y: 400, width: 300, height: 200 });
  value = nudgedGeometry(value, -1, -1, 1000, 600); assert.equal(value.x, 699); assert.equal(value.y, 399);
  assert.equal(nudgedGeometry(value, -1000, -1000, 1000, 600).x, 0);
  checks.push('Each 1/10 source-pixel intent clamps immediately; reversing at an edge does not retain hidden overshoot');
}
{
  const f = fixture(); f.gate.nudge(f.target(), 1, 0, true); await until(() => f.calls.length === 1);
  for (let i = 0; i < 100; i++) f.gate.nudge(f.target(), 1, 0, false);
  assert.equal(f.calls.length, 1); assert.equal(f.view.overlay.geometry.x, 201); assert.equal(f.operations, 1);
  const flushed = f.gate.flush();
  f.finish(0); await tick(); assert.equal(f.calls.length, 2); assert.equal(f.calls[1].expectedRevision, 1); assert.equal(f.calls[1].from.x, 101); assert.equal(f.calls[1].to.x, 201);
  f.finish(1); await flushed; assert.equal(f.operations, 0); assert.equal(f.view.pending, false); assert.equal(f.calls[0].editId, f.calls[1].editId); assert.equal(f.finishes.length, 1); f.gate.dispose();
  checks.push('100 repeat events coalesce behind one in-flight commit; receipt drives the next CAS and flush waits for the latest intent');
}
{
  const f = fixture(); f.gate.nudge(f.target(), 1, 0, true); f.gate.endKeys(false); const flushed = f.gate.flush(); await tick();
  assert.equal(f.gate.nudge(f.target(), 10, 0, true), true); f.gate.endKeys(false); f.finish(0); await tick(); assert.equal(f.calls[1].to.x, 111); assert.notEqual(f.calls[0].editId, f.calls[1].editId); assert.equal(f.finishes.length, 1); f.finish(1); await flushed; assert.equal(f.finishes.length, 2); f.gate.dispose();
  checks.push('A fresh key press during the previous key-up drain joins the latest target instead of being dropped');
}
{
  const f = fixture(); f.gate.nudge(f.target(), 1, 0, true); await until(() => f.calls.length === 1); f.waits[0].reject(Error('save failed')); await until(() => f.errors.length === 1); await assert.rejects(f.gate.flush(), /save failed/);
  for (let i = 0; i < 10; i++) assert.equal(f.gate.nudge(f.target(), 1, 0, false), false);
  assert.equal(f.calls.length, 1); assert.equal(f.view.overlay, undefined); assert.equal(f.scene.regions[0].x, 100); assert.equal(f.operations, 0);
  f.gate.endKeys(false); assert.equal(f.gate.nudge(f.target(), 1, 0, true), true); const retry = f.gate.flush(); await tick(); f.finish(1); await retry; f.gate.dispose();
  checks.push('A failed held burst restores persisted geometry and cannot auto-retry; releasing and pressing again permits an explicit retry');
}
{
  const f = fixture(); const region = f.scene.regions[0]; region.imageOverride = { id: 'long' };
  f.gate.beginPointer(f.target()); f.gate.previewPointer({ x: 160, y: 140, width: 300, height: 200 });
  region.translation = { overlay: { id: 'new-translation' } }; region.ocr = { document: { text: 'new OCR' } }; f.gate.reconcile();
  const shown = overlayRegion(f.scene, region, f.view.overlay); assert.equal(shown.translation, region.translation); assert.equal(shown.ocr, region.ocr); assert.equal(shown.drawings, region.drawings); assert.equal(shown.x, 160);
  const flushed = f.gate.endPointer(true); await tick(); assert.deepEqual(Object.keys(f.calls[0]).sort(), ['backgroundId', 'editId', 'expectedRevision', 'from', 'regionId', 'sceneId', 'sourceId', 'to', 'type'].sort()); assert.equal(f.calls[0].sourceId, 'long');
  f.finish(0); await flushed; assert.equal(region.translation.overlay.id, 'new-translation'); f.gate.dispose();
  checks.push('Pointer preview only overlays geometry; newly arriving override OCR/translation/history remain live and are never submitted back');
}
{
  const f = fixture(); f.gate.nudge(f.target(), 1, 0, true); await until(() => f.calls.length === 1); f.gate.nudge(f.target(), 10, 0, false); const failure = assert.rejects(f.gate.flush(), /区域位置已变化/); f.finish(0, true); await failure;
  assert.equal(f.calls.length, 1); assert.equal(f.scene.regions[0].x, 102); assert.equal(f.view.overlay, undefined); f.gate.dispose();
  checks.push('A later external snapshot cannot be adopted as our receipt; no queued intent replays onto its revision');
}
{
  const f = fixture(); f.gate.nudge(f.target(), 1, 0, true); const flushed = f.gate.flush(); await tick(); const receipt = { ...f.target(), geometry: f.calls[0].to, revision: 1 }; f.scene = { ...makeScene(), id: 'other' }; f.waits[0].resolve(receipt); await flushed;
  assert.equal(f.view.overlay, undefined); assert.equal(f.operations, 0); assert.deepEqual(f.errors, []); f.gate.dispose();
  checks.push('Scene/source invalidation rejects late overlay publication and releases the operation barrier');
}
{
  const f = fixture(); f.gate.beginPointer(f.target()); f.gate.previewPointer({ ...f.target().geometry, x: 120 }); f.gate.pauseInput(true); await f.gate.flush(); assert.equal(f.calls.length, 0); assert.equal(f.operations, 0);
  f.gate.pauseInput(false); f.gate.nudge(f.target(), 10, 0, true); f.gate.pauseInput(true); const flushed = f.gate.flush(); await tick(); assert.equal(f.gate.nudge(f.target(), 1, 0, true), false); f.finish(0); await flushed; assert.equal(f.operations, 0); f.gate.dispose();
  checks.push('Exit cancels incomplete pointer gestures but drains accepted keyboard intent before the general operation barrier');
}
{
  const f = fixture(); const current = f.target(), command = { ...f.calls[0], type: 'set_region_geometry', sceneId: 'scene', regionId: 'r', backgroundId: 'bg', sourceId: 'bg', expectedRevision: 0, from: current.geometry, to: current.geometry };
  assert.equal(geometryReceipt({ scenes: [f.scene] }, command).revision, 0);
  assert.throws(() => geometryReceipt({ scenes: [f.scene] }, { ...command, sourceId: 'wrong' }));
  assert.throws(() => geometryReceipt({ scenes: [f.scene] }, { ...command, to: { ...command.to, x: 200 } })); f.gate.dispose();
  checks.push('Commit receipts require the exact source, destination and own revision; no-op commits retain revision');
}

// Actual browser bridge in a separate VM module with an explicit synthetic store seed.
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [name, value] of Object.entries(values)) this.setExport(name, value); });
const policy = new SourceTextModule(await source('./connection-policy.ts')); await policy.link(() => { throw Error('Unexpected policy dependency'); }); await policy.evaluate();
const mosaic = new SourceTextModule(await source('./mosaic-preview.ts')); await mosaic.link(() => { throw Error('Unexpected mosaic dependency'); }); await mosaic.evaluate();
const history = await loadGeometryHistory(); const importedAssets = await loadAssetImport();
if (!globalThis.crypto) globalThis.crypto = webcrypto;
const bridge = new SourceTextModule(`${await source('./bridge.ts')}\nexport const seed = value => { preview = value; };`, { initializeImportMeta: meta => { meta.glob = () => ({}); } });
await bridge.link(async name => name === './provider-presets' ? await loadProviderPresets() : name === './asset-import' ? importedAssets : name === './region-geometry-history' ? history : name === './connection-policy' ? policy : name === './mosaic-preview' ? mosaic : name.endsWith('/core') ? synthetic({ invoke: () => { throw Error('No native IPC allowed'); }, isTauri: () => false }) : synthetic({ listen: () => { throw Error('No native events allowed'); } })); await bridge.evaluate();
{
  const scene = makeScene(), region = scene.regions[0], state = { schemaVersion: 1, activeSceneId: scene.id, scenes: [scene], agents: [], memories: [], memoryStats: [], mcpServers: [], connections: [], defaultConnectionId: null };
  bridge.namespace.seed(state);
  const command = { type: 'set_region_geometry', sceneId: scene.id, regionId: 'r', backgroundId: 'bg', sourceId: 'bg', expectedRevision: 0, from: { x: 100, y: 120, width: 300, height: 200 }, to: { x: 101, y: 120, width: 300, height: 200 } };
  region.ocr = { drawingRevision: 0 }; region.translation = { drawingRevision: 0 };
  await bridge.namespace.applyCommand(command); assert.equal(region.ocr, undefined); assert.equal(region.translation, undefined); assert.equal(region.drawings[0].id, 'draw');
  await assert.rejects(bridge.namespace.applyCommand(command), /区域位置已变化/);
  region.imageOverride = { id: 'long' }; region.ocr = { drawingRevision: 1, document: 'new OCR' }; region.translation = { drawingRevision: 1, overlay: 'new translation' };
  const moved = { ...command, sourceId: 'long', expectedRevision: 1, from: command.to, to: { ...command.to, y: 130 } };
  await bridge.namespace.applyCommand(moved); assert.equal(region.ocr.document, 'new OCR'); assert.equal(region.ocr.drawingRevision, 2); assert.equal(region.translation.overlay, 'new translation'); assert.equal(region.translation.drawingRevision, 2);
  const noop = { ...moved, expectedRevision: 2, from: moved.to, to: moved.to }; await bridge.namespace.applyCommand(noop); assert.equal(region.drawingRevision, 2);
  for (const bad of [{ sourceId: 'bg' }, { expectedRevision: 1 }, { from: command.from }, { to: { ...moved.to, x: 999 } }, { to: { ...moved.to, x: NaN } }]) await assert.rejects(bridge.namespace.applyCommand({ ...noop, ...bad }));
  for (const mode of ['closed', 'frozen', 'running', 'inactive']) { scene.closed = mode === 'closed'; scene.frozen = mode === 'frozen'; scene.run = mode === 'running' ? { status: 'running' } : undefined; state.activeSceneId = mode === 'inactive' ? 'other' : scene.id; await assert.rejects(bridge.namespace.applyCommand(noop)); }
  await assert.rejects(bridge.namespace.applyCommand({ type: 'update_region', sceneId: scene.id, region }), /区域几何/);
  checks.push('Actual preview bridge mirrors CAS/no-op/bounds/active guards and preserves override derived content while rejecting legacy whole-region writes');
}
const submission = new SourceTextModule(await source('./scene-submission.ts')); await submission.link(() => { throw Error('Unexpected submission import'); }); await submission.evaluate();
{
  const waitGeometry = deferred(), waitSend = deferred(), order = [], persisted = { draft: '' }; let localDraft = 'A', sentDraft, queue = Promise.resolve();
  const run = (async () => {
    await waitGeometry.promise;
    const submittedDraft = localDraft;
    const next = queue.then(() => submission.namespace.submitSceneDraft({ sceneId: 'scene', draft: submittedDraft, refs: [] }, {
      save: async command => { order.push(command.type); if (command.type === 'set_draft') persisted.draft = command.draft; },
      send: async () => { order.push('send'); sentDraft = persisted.draft; await waitSend.promise; return { scenes: [{ id: 'scene', draft: '' }] }; },
    })); queue = next.then(() => {});
    const result = await next;
    if (localDraft === submittedDraft) localDraft = result.scenes[0].draft;
  })();
  localDraft = 'B'; waitGeometry.resolve(); await tick(); assert.equal(sentDraft, 'B');
  localDraft = 'C'; queue = queue.then(() => { order.push('autosave C'); persisted.draft = localDraft; });
  waitSend.resolve(); await run; await queue; assert.equal(localDraft, 'C'); assert.equal(persisted.draft, 'C'); assert.deepEqual(order, ['set_draft', 'set_refs', 'send', 'autosave C']);
  checks.push('Send captures after slow geometry, serializes draft/refs/begin_run, and preserves later input without autosave entering the submitted payload');
}
{
  let sent = false;
  await assert.rejects(submission.namespace.submitSceneDraft({ sceneId: 'scene', draft: 'kept', refs: [] }, { save: async command => { if (command.type === 'set_refs') throw Error('refs failed'); }, send: async () => { sent = true; return {}; } }), /refs failed/);
  assert.equal(sent, false);
  checks.push('A failed draft/reference save cannot proceed to the model request or a successful clear');
}
{
  const wait = deferred(), f = fixture({ finish: () => wait.promise });
  f.gate.nudge(f.target(), 1, 0, true); f.gate.endKeys(false); const flushed = f.gate.flush(); await tick();
  f.gate.nudge(f.target(), 10, 0, true); f.gate.endKeys(false); f.finish(0); await tick();
  assert.equal(f.calls.length, 1); assert.equal(f.operations, 1); assert.equal(f.finishes.length, 1);
  wait.resolve(); await tick(); assert.equal(f.calls.length, 2); assert.notEqual(f.calls[0].editId, f.calls[1].editId); f.finish(1); await flushed; assert.equal(f.operations, 0); f.gate.dispose();
  checks.push('A delayed Finish acknowledgment remains inside the operation barrier; the next fresh gesture cannot submit or reuse the sealed editId before it');
}
{
  const f = fixture(); let accepted = 0;
  for (let i = 0; i < 100; i++) { if (f.gate.nudge(f.target(), 1, 0, true)) accepted++; f.gate.endKeys(false); }
  assert.equal(accepted, 50); assert.equal(f.errors.length, 1); assert.equal(f.view.overlay.geometry.x, 150);
  let done = false; const flushed = f.gate.flush().then(() => { done = true; }); let committed = 0;
  for (let i = 0; !done && i < 200; i++) { await tick(); if (f.calls.length > committed) f.finish(committed++); }
  await flushed; assert.equal(committed, 50); assert.equal(f.finishes.length, 50); assert.equal(new Set(f.calls.map(v => v.editId)).size, 50); assert.equal(f.scene.regions[0].x, 150); assert.equal(f.operations, 0); f.gate.dispose();
  checks.push('100 rapid down/up bursts admit at most 50 boundaries with one capacity notice; all accepted endpoints finish in order without overwriting or unbounded queuing');
}
{
  const f = fixture(); f.scene.regions[0].x = 700;
  for (let i = 0; i < 100; i++) { assert.equal(f.gate.nudge(f.target(), 10, 0, true), true); f.gate.endKeys(false); }
  await f.gate.flush(); assert.equal(f.calls.length, 0); assert.equal(f.finishes.length, 0); assert.equal(f.operations, 0); f.gate.dispose();
  checks.push('Clamped movement at the edge creates neither an empty native edit nor a Finish/history operation');
}
{
  const wait = deferred(), f = fixture({ finish: () => wait.promise });
  f.gate.nudge(f.target(), 1, 0, true); f.gate.endKeys(false); const a = assert.rejects(f.gate.flush(), /finish failed/), b = assert.rejects(f.gate.flush(), /finish failed/); await tick();
  f.gate.nudge(f.target(), 10, 0, true); f.gate.endKeys(false); f.finish(0); await tick(); wait.reject(Error('finish failed')); await Promise.all([a, b]);
  assert.equal(f.calls.length, 1); assert.equal(f.scene.regions[0].x, 101); assert.equal(f.view.overlay, undefined); assert.equal(f.operations, 0); f.gate.dispose();
  checks.push('Finish failure rejects every concurrent flush waiter, preserves the last persisted endpoint and drops later uncommitted groups');
}
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
