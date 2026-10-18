// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule } from 'node:vm';

const diagnosticSource = await readFile(new URL('./capture-performance.ts', import.meta.url), 'utf8');
const presentationSource = await readFile(new URL('./capture-presentation.ts', import.meta.url), 'utf8');
async function load(source, globals = {}) {
  const module = new SourceTextModule(stripTypeScriptTypes(source, { mode: 'transform' }), { context: createContext(globals) });
  await module.link(() => { throw new Error('Unexpected runtime import'); }); await module.evaluate(); return module.namespace;
}
async function fixture() {
  let now = 0, calls = 0;
  const api = await load(diagnosticSource, { performance: { now: () => { calls++; return now; } }, document: { visibilityState: 'hidden' } });
  const identity = { sceneId: 'scene', backgroundId: 'background', width: 1920, height: 1080 };
  const snapshot = { activeSceneId: identity.sceneId, scenes: [{ id: identity.sceneId, frozen: false, background: { id: identity.backgroundId, width: identity.width, height: identity.height } }] };
  return { api, identity, snapshot, time: value => { now = value; }, calls: () => calls };
}
function normalized(f) { const token = f.api.captureSnapshotReceived(f.snapshot); f.time(1); f.api.captureSnapshotNormalized(token); return token; }
function complete(f, session) { for (const [i, phase] of ['attached', 'decodeStarted', 'imageDecoded', 'layoutReady'].entries()) { f.time(i + 2); session.mark(phase); } f.time(6); return session.finish(); }

test('default and explicitly disabled capture measurement read no clock, document or snapshot', async () => {
  let reads = 0;
  const source = new Proxy({}, { get() { reads++; throw Error('Unexpected business read'); } });
  const api = await load(diagnosticSource, { performance: { now() { throw Error('Unexpected clock'); } } });
  assert.equal(api.captureSnapshotReceived(source), undefined);
  api.captureSnapshotNormalized(undefined);
  assert.equal(api.capturePreparationPerformance({ sceneId: 'x', backgroundId: 'y', width: 1, height: 1 }), undefined);
  api.configureCapturePerformance(false); assert.equal(api.captureSnapshotReceived(source), undefined); assert.equal(reads, 0);
});
test('a measured hidden snapshot reports exactly seven monotonic fixed numeric stages without source data', async () => {
  const f = await fixture(); f.api.configureCapturePerformance(true); normalized(f);
  const result = complete(f, f.api.capturePreparationPerformance(f.identity));
  assert.deepEqual(JSON.parse(JSON.stringify(result)), { documentHidden: true, receivedMicros: 0, normalizedMicros: 1000, attachedMicros: 2000, decodeStartedMicros: 3000, imageDecodedMicros: 4000, layoutReadyMicros: 5000, invokeStartedMicros: 6000 });
  assert.equal(f.calls(), 7); assert.equal(f.api.capturePreparationPerformance(f.identity), undefined);
  assert.equal(f.api.captureSnapshotReceived(f.snapshot), undefined); assert.equal(f.calls(), 7);
});
test('replaced identities and late normalization cannot mark or finish the new source', async () => {
  const f = await fixture(); f.api.configureCapturePerformance(true); const oldToken = normalized(f), old = f.api.capturePreparationPerformance(f.identity);
  f.time(2); const next = structuredClone(f.snapshot); next.scenes[0].background.id = 'new';
  const current = f.api.captureSnapshotReceived(next); f.api.captureSnapshotNormalized(oldToken);
  assert.equal(f.api.capturePreparationPerformance({ ...f.identity, backgroundId: 'new' }), undefined);
  f.time(3); f.api.captureSnapshotNormalized(current);
  old.mark('attached'); assert.equal(old.finish(), undefined);
  const session = f.api.capturePreparationPerformance({ ...f.identity, backgroundId: 'new' });
  for (const [i, phase] of ['attached', 'decodeStarted', 'imageDecoded', 'layoutReady'].entries()) { f.time(i + 4); session.mark(phase); }
  f.time(8); assert.equal(session.finish().invokeStartedMicros, 6000);
});
test('disabling measurement or closing the active source invalidates in-flight diagnostics without extra clocks', async () => {
  for (const close of [false, true]) {
    const f = await fixture(); f.api.configureCapturePerformance(true); normalized(f); const session = f.api.capturePreparationPerformance(f.identity), count = f.calls();
    if (close) f.api.captureSnapshotReceived({ ...f.snapshot, scenes: [{ ...f.snapshot.scenes[0], closed: true }] }); else f.api.configureCapturePerformance(false);
    session.mark('attached'); assert.equal(session.finish(), undefined); assert.equal(f.calls(), count);
  }
});
test('nonfinite, reversed, excessive and out-of-order clocks silently discard advisory timing', async () => {
  for (const invalid of [NaN, Infinity, -1, 8001]) {
    const f = await fixture(); f.api.configureCapturePerformance(true); normalized(f); const session = f.api.capturePreparationPerformance(f.identity);
    f.time(invalid); assert.doesNotThrow(() => session.mark('attached')); assert.equal(session.finish(), undefined);
  }
  const f = await fixture(); f.api.configureCapturePerformance(true); normalized(f); const session = f.api.capturePreparationPerformance(f.identity);
  session.mark('imageDecoded'); assert.equal(session.finish(), undefined);
});
test('the production readiness helper keeps one decode/factory/ACK and carries the measured stages', async () => {
  const f = await fixture(); f.api.configureCapturePerformance(true); normalized(f); const api = await load(presentationSource);
  const events = [], reports = []; let release, factories = 0;
  const image = { isConnected: true, dataset: { asset: 'background' }, getAttribute: () => '/synthetic.png', complete: true, naturalWidth: 1920, naturalHeight: 1080,
    decode: () => { events.push('decode'); f.time(4); return new Promise(resolve => { release = resolve; }); },
    getBoundingClientRect: () => { events.push('layout'); f.time(5); return { width: 1920, height: 1080 }; } };
  const factory = () => { factories++; return f.api.capturePreparationPerformance(f.identity); };
  f.time(2); const first = api.prepareCaptureBackground(image, f.identity, () => true, async value => reports.push(value), factory);
  const duplicate = api.prepareCaptureBackground(image, f.identity, () => true, async value => reports.push(value), factory);
  assert.equal(first, duplicate); await Promise.resolve(); release(); await first;
  assert.equal(factories, 1); assert.equal(reports.length, 1); assert.equal(reports[0].success, true);
  assert.deepEqual(events, ['decode', 'layout']); assert.equal(reports[0].diagnostics.imageDecodedMicros, 4000);
  assert.equal(reports[0].diagnostics.invokeStartedMicros, 5000);
});
test('throwing optional diagnostic factories and hooks cannot fail or delay capture readiness', async () => {
  const api = await load(presentationSource), identity = { sceneId: 'scene', backgroundId: 'background', width: 2, height: 3 };
  for (const factory of [() => { throw Error('advisory'); }, () => ({ mark() { throw Error('advisory'); }, finish() { throw Error('advisory'); } })]) {
    const image = { isConnected: true, dataset: { asset: 'background' }, getAttribute: () => '/synthetic.png', complete: true, naturalWidth: 2, naturalHeight: 3, decode: async () => {}, getBoundingClientRect: () => ({ width: 2, height: 3 }) };
    const reports = []; await api.prepareCaptureBackground(image, identity, () => true, async value => reports.push(value), factory);
    assert.deepEqual(JSON.parse(JSON.stringify(reports)), [{ ...identity, success: true }]);
  }
});
test('failed decoding emits the original failure without frontend diagnostics', async () => {
  const f = await fixture(); f.api.configureCapturePerformance(true); normalized(f); const api = await load(presentationSource), reports = [];
  const image = { isConnected: true, dataset: { asset: 'background' }, getAttribute: () => '/synthetic.png', decode: async () => { throw Error('decode'); } };
  await api.prepareCaptureBackground(image, f.identity, () => true, async value => reports.push(value), () => f.api.capturePreparationPerformance(f.identity));
  assert.deepEqual(JSON.parse(JSON.stringify(reports)), [{ ...f.identity, success: false }]);
});
