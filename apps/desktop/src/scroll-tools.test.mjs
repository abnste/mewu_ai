// node --experimental-vm-modules apps/desktop/src/scroll-tools.test.mjs
// Isolated events, projection and IPC; no screen, process or clipboard access.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule, createContext } from 'node:vm';
import { webcrypto } from 'node:crypto';
import { loadProviderPresets, loadGeometryHistory, loadAssetImport } from './geometry-test-module.mjs';
const source = async file => stripTypeScriptTypes(await readFile(new URL(file, import.meta.url), 'utf8'), { mode: 'transform' });
const plain = value => JSON.parse(JSON.stringify(value));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const tick = () => new Promise(resolve => setImmediate(resolve));
const synthetic = (values, context) => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, context ? { context } : undefined);
const checks = [];

const projection = new SourceTextModule(await source('region-image.ts')); await projection.link(() => { throw new Error('unexpected import'); }); await projection.evaluate();
const { regionImageProjection } = projection.namespace;
const background = { id: 'desktop', width: 1920, height: 1080 }, box = { x: 240, y: 140, width: 400, height: 500 };
const region = { id: 'region', x: 320, y: 160, width: 800, height: 500, drawingRevision: 4, drawings: [{ id: 'line', points: [{ x: 0, y: 40 }, { x: 600, y: 40 }] }], imageOverride: { id: 'long', width: 600, height: 2400 } };
const shown = regionImageProjection(region, box, background, .5);
assert.deepEqual(plain(shown.box), { x: 377.5, y: 140, width: 125, height: 500 });
assert.deepEqual([shown.region.x, shown.region.y, shown.region.width, shown.region.height], [0, 0, 600, 2400]);
assert.equal(shown.region.drawings, region.drawings); assert.equal(shown.scale, 500 / 2400);
assert.equal(region.x, 320); assert.equal(region.width, 800); assert.equal(shown.region.imageOverride.id, 'long');
const wide = regionImageProjection({ ...region, imageOverride: { id: 'wide', width: 2400, height: 600 } }, box, background, .5);
assert.deepEqual(plain(wide.box), { x: 240, y: 340, width: 400, height: 100 });
const normal = { ...region, imageOverride: undefined }; assert.equal(regionImageProjection(normal, box, background, .5).region, normal);
checks.push('Tall/wide images contain without stretching; virtual drawing coordinates keep source pixels and cannot mutate persistent screen geometry');

async function scrollFixture(native = true, query = Promise.resolve(null)) {
  const calls = [], stops = [], events = new Map();
  const module = new SourceTextModule(await source('scroll-bridge.ts'));
  await module.link(name => name.endsWith('/core') ? synthetic({ isTauri: () => native, invoke: async (command, args) => { calls.push({ command, args }); if (command === 'get_scroll_status') return query; } }) : synthetic({ listen: async (name, callback) => { events.set(name, callback); return () => stops.push(name); } }));
  await module.evaluate(); return { api: module.namespace, calls, stops, events };
}
const ready = await scrollFixture(), { ScrollStatusGate, scrollNeedsKeep } = ready.api;
const initial = { id: 'one', sceneId: 'scene', phase: 'starting', rect: { x: .1, y: .2, width: .4, height: .5 }, width: 800, height: 500, status: 'initial', stopHotkey: 'F8 / Esc' };
const gate = new ScrollStatusGate(); assert.equal(gate.accept(initial), true);
assert.equal(gate.accept({ ...initial, phase: 'capturing', status: 'extended', height: 1000 }), true);
assert.equal(gate.accept(initial), false); assert.equal(gate.accept({ ...initial, phase: 'finishing' }), true);
assert.equal(gate.accept({ ...initial, phase: 'capturing' }), false); assert.equal(gate.accept(null), true); assert.equal(gate.accept(initial), false);
assert.equal(gate.accept({ ...initial, id: 'two' }), true); gate.dispose(); assert.equal(gate.accept(null), false);
for (const status of ['low_information', 'ambiguous', 'lost_overlap', 'limit_reached']) assert.equal(scrollNeedsKeep({ ...initial, status }), true);
for (const status of ['initial', 'extended', 'retraced', 'unchanged']) assert.equal(scrollNeedsKeep({ ...initial, status }), false);
checks.push('Status gate rejects phase regressions, ended sessions and unmounted events; partial-output states require explicit keep');

const delayed = deferred(), raced = await scrollFixture(true, delayed.promise), received = [];
const subscribing = raced.api.subscribeScroll(value => received.push(plain(value))); await tick();
assert.equal(raced.events.has('scroll-status'), true); raced.events.get('scroll-status')({ payload: { ...initial, phase: 'capturing' } }); raced.events.get('scroll-status')({ payload: null });
delayed.resolve(initial); const stop = await subscribing;
assert.deepEqual(received.map(value => value?.phase ?? null), ['capturing', null]);
stop(); raced.events.get('scroll-status')({ payload: { ...initial, id: 'late' } }); assert.equal(received.length, 2); assert.deepEqual(raced.stops, ['scroll-status']);
const failure = deferred(), failed = await scrollFixture(true, failure.promise), rejected = assert.rejects(failed.api.subscribeScroll(() => {}), /query failed/); await tick(); failure.reject(new Error('query failed')); await rejected; assert.deepEqual(failed.stops, ['scroll-status']);
checks.push('Listen-before-query cannot resurrect a completed capture; disposal/query failure release listener and reject late events');

const target = { sceneId: 'scene', regionId: 'region', backgroundId: 'desktop', drawingRevision: 4, x: 320, y: 160, width: 800, height: 500 };
await ready.api.startScrollCapture('mewu.scroll', 2, 'capture', target); await ready.api.controlScrollCapture('one', 'keep'); await ready.api.controlScrollCapture('one', 'finish');
assert.deepEqual(plain(ready.calls), [{ command: 'start_scroll_capture', args: { pluginId: 'mewu.scroll', revision: 2, contributionId: 'capture', target } }, { command: 'control_scroll_capture', args: { id: 'one', action: 'keep' } }, { command: 'control_scroll_capture', args: { id: 'one', action: 'finish' } }]);
const browser = await scrollFixture(false); await assert.rejects(browser.api.startScrollCapture('x', 1, 'x', target), /桌面版/); await assert.rejects(browser.api.controlScrollCapture('one', 'finish'), /桌面版/); const noEvents = []; (await browser.api.subscribeScroll(value => noEvents.push(value)))(); assert.deepEqual(noEvents, [null]); assert.equal(browser.calls.length, 0);
checks.push('Exact IPC keeps original screen geometry/background identity; browser never fabricates scroll capture or completion');

async function mainBridgeFixture(native, probe) {
  const calls = [], context = createContext({ crypto: webcrypto, structuredClone, Date, Map, Set, URL, Blob, setTimeout, clearTimeout, Image: class { naturalWidth = 100; naturalHeight = 200; decode() { return Promise.resolve(); } } });
  const core = synthetic({ isTauri: () => native, invoke: async (command, args) => { calls.push({ command, args }); if (command === 'get_mosaic_preview') { if (probe) await probe(args); return { backgroundId: args.sourceId ?? args.backgroundId, width: 1200, height: 2400, blockSize: 12, columns: 100, rows: 200, dataUrl: 'data:image/png;base64,eA==' }; } } }, context);
  const policy = new SourceTextModule(await source('connection-policy.ts'), { context }); await policy.link(() => { throw new Error('unexpected'); });
  const history = await loadGeometryHistory(context); const importedAssets = await loadAssetImport(context);
  const mosaic = new SourceTextModule(await source('mosaic-preview.ts'), { context }); await mosaic.link(() => { throw new Error('unexpected'); });
  const module = new SourceTextModule(await source('bridge.ts') + '\nexport function seedBackground(value) { findScene(preview.activeSceneId).background = value; }', { context, initializeImportMeta: meta => { meta.glob = () => ({}); } });
  await module.link(async name => name === './provider-presets' ? await loadProviderPresets(context) : name === './asset-import' ? importedAssets : name === './region-geometry-history' ? history : name === './connection-policy' ? policy : name === './mosaic-preview' ? mosaic : name.endsWith('/core') ? core : synthetic({ listen: async () => () => {} }, context)); await module.evaluate();
  return { api: module.namespace, calls };
}
const main = await mainBridgeFixture(true), image = { id: 'long', width: 1200, height: 2400, kind: 'image', path: '/long.png', name: 'long' };
await main.api.getMosaicPreview('scene', background, 12, { regionId: 'region', source: image, drawingRevision: 4 });
await main.api.getMosaicPreview('scene', background, 12, { regionId: 'region', source: image, drawingRevision: 5 });
await main.api.getMosaicPreview('scene', background, 12);
assert.equal(main.calls.length, 2); assert.deepEqual(plain(main.calls[0].args), { sceneId: 'scene', backgroundId: 'desktop', blockSize: 12, regionId: 'region', sourceId: 'long', drawingRevision: 4 });
assert.equal(main.calls[1].args.sourceId, undefined);
checks.push('Mosaic uses source-specific cache and original-background CAS; source pixels are cached across drawing revisions without mixing desktop and long image');

const oldGrid = deferred(), newGrid = deferred(), racing = await mainBridgeFixture(true, args => args.drawingRevision === 4 ? oldGrid.promise : newGrid.promise);
const obsolete = racing.api.getMosaicPreview('scene', background, 12, { regionId: 'region', source: image, drawingRevision: 4 });
const obsoleteError = assert.rejects(obsolete, /stale revision/);
const current = racing.api.getMosaicPreview('scene', background, 12, { regionId: 'region', source: image, drawingRevision: 5 });
assert.notEqual(obsolete, current);
assert.equal(current, racing.api.getMosaicPreview('scene', background, 12, { regionId: 'region', source: image, drawingRevision: 5 }));
await tick(); assert.equal(racing.calls.length, 2);
newGrid.resolve(); await current; oldGrid.reject(new Error('stale revision')); await obsoleteError;
await racing.api.getMosaicPreview('scene', background, 12, { regionId: 'region', source: image, drawingRevision: 6 }); assert.equal(racing.calls.length, 2);
checks.push('Concurrent old/new revisions never share a rejected CAS promise; late old failure cannot delete the successful immutable grid or prevent later revision reuse');

const local = await mainBridgeFixture(false), snapshot = await local.api.getSnapshot(), sceneId = snapshot.activeSceneId;
local.api.seedBackground(background);
await local.api.applyCommand({ type: 'add_region', sceneId, region: { ...region, imageOverride: image } });
let next = await local.api.getSnapshot(), saved = next.scenes.find(value => value.id === sceneId).regions[0]; assert.equal(saved.imageOverride, undefined);
await assert.rejects(local.api.applyCommand({ type: 'update_region', sceneId, region: { ...saved, x: 500, imageOverride: image, drawingRevision: 999, drawings: [] } }), /区域几何/);
next = await local.api.applyCommand({ type: 'set_region_geometry', sceneId, regionId: saved.id, backgroundId: background.id, sourceId: background.id, expectedRevision: 4, from: { x: saved.x, y: saved.y, width: saved.width, height: saved.height }, to: { x: 500, y: saved.y, width: saved.width, height: saved.height } });
saved = next.scenes.find(value => value.id === sceneId).regions[0]; assert.equal(saved.drawingRevision, 5); assert.equal(saved.imageOverride, undefined); assert.deepEqual(plain(saved.drawings), region.drawings);
checks.push('Browser typed geometry CAS preserves host-owned drawings and advances its revision; legacy whole-region writes cannot inject an override or replace drawings');
const config = JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const controlCapability = JSON.parse(await readFile(new URL('../src-tauri/capabilities/scroll.json', import.meta.url), 'utf8'));
const spaceCapability = JSON.parse(await readFile(new URL('../src-tauri/capabilities/space.json', import.meta.url), 'utf8'));
assert.ok(config.app.security.capabilities.includes(controlCapability.identifier), 'explicit capability list must reference identifier, not filename');
assert.deepEqual(controlCapability.windows, ['scroll-controls']);
assert.deepEqual([...controlCapability.permissions].sort(), ['allow-control-scroll-capture', 'allow-get-scroll-status', 'core:event:allow-listen', 'core:event:allow-unlisten'].sort());
for (const permission of ['allow-start-scroll-capture', 'allow-get-scroll-status', 'allow-control-scroll-capture']) assert.ok(spaceCapability.permissions.includes(permission));
checks.push('Native config explicitly enables the actual scroll-controls capability identifier; control window is least-privileged and space owns start/get/control');
console.log(JSON.stringify({ passed: true, checks }, null, 2));
