// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile, readdir } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
const source = await readFile(new URL('./capture-presentation.ts', import.meta.url), 'utf8');
const { prepareCaptureBackground } = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source, { mode: 'transform' })).toString('base64')}`);
const identity = { sceneId: 'scene', backgroundId: 'new', width: 1920, height: 1080 };
function fixture() {
  const events = [], reports = [];
  let release;
  const decoded = new Promise(resolve => { release = resolve; });
  const image = { isConnected: true, dataset: { asset: 'new' }, source: '/full-screen.png', getAttribute: () => image.source, complete: true, naturalWidth: 1920, naturalHeight: 1080,
    decode: () => { events.push('decode'); return decoded; },
    getBoundingClientRect: () => { events.push('layout'); return { width: 1536, height: 864 }; } };
  const report = async value => { events.push('report'); reports.push(value); };
  return { image, events, reports, report, release };
}
test('a hidden screenshot is acknowledged after decode and synchronous layout, without a frame timer or resampling', async () => {
  const f = fixture();
  const ready = prepareCaptureBackground(f.image, identity, () => true, f.report);
  await Promise.resolve(); assert.deepEqual(f.events, ['decode']); assert.equal(f.reports.length, 0);
  f.release(); await ready;
  assert.deepEqual(f.events, ['decode', 'layout', 'report']);
  assert.deepEqual(f.reports, [{ ...identity, success: true }]);
});
test('switching scenes while decode is pending never acknowledges an old image', async () => {
  const f = fixture(); let current = true;
  const ready = prepareCaptureBackground(f.image, identity, () => current, f.report);
  current = false; f.release(); await ready; assert.equal(f.reports.length, 0);
});
test('a disconnected or replaced background cannot wake capture', async () => {
  for (const replacement of ['disconnected', 'replaced']) {
    const f = fixture(), ready = prepareCaptureBackground(f.image, identity, () => true, f.report);
    if (replacement === 'disconnected') f.image.isConnected = false; else f.image.dataset.asset = 'other';
    f.release(); await ready; assert.equal(f.reports.length, 0);
  }
});
test('decode errors are failed acknowledgements, never inferred successes', async () => {
  const f = fixture(); f.image.decode = async () => { throw Error('decode'); };
  await prepareCaptureBackground(f.image, identity, () => true, f.report);
  assert.deepEqual(f.reports, [{ ...identity, success: false }]); assert.deepEqual(f.events, ['report']);
});
test('attach, load and error callbacks share exactly one decode and acknowledgement for the current element', async () => {
  const f = fixture();
  const first = prepareCaptureBackground(f.image, identity, () => true, f.report);
  const duplicate = prepareCaptureBackground(f.image, { ...identity }, () => true, f.report);
  assert.equal(first, duplicate);
  await Promise.resolve(); assert.deepEqual(f.events, ['decode']);
  f.release(); await Promise.all([first, duplicate]);
  await prepareCaptureBackground(f.image, identity, () => true, f.report);
  assert.deepEqual(f.events, ['decode', 'layout', 'report']);
  assert.deepEqual(f.reports, [{ ...identity, success: true }]);
});
test('both thrown and rejected decodes report failure once, even if the load/error path also runs', async () => {
  for (const synchronous of [false, true]) {
    const f = fixture(); let count = 0;
    f.image.decode = () => { count++; if (synchronous) throw Error('decode'); return Promise.reject(Error('decode')); };
    await Promise.all([prepareCaptureBackground(f.image, identity, () => true, f.report),
      prepareCaptureBackground(f.image, identity, () => true, f.report)]);
    await prepareCaptureBackground(f.image, identity, () => true, f.report);
    assert.equal(count, 1); assert.deepEqual(f.reports, [{ ...identity, success: false }]);
  }
});
test('a source replacement invalidates the pending preparation even if the old URL is restored', async () => {
  const f = fixture(); let resolveSecond;
  const first = prepareCaptureBackground(f.image, identity, () => true, f.report);
  await Promise.resolve();
  f.image.source = '/replacement.png';
  f.image.decode = () => new Promise(resolve => { resolveSecond = resolve; });
  const second = prepareCaptureBackground(f.image, identity, () => true, f.report);
  await Promise.resolve();
  f.image.source = '/full-screen.png'; f.release(); resolveSecond();
  await Promise.all([first, second]); assert.equal(f.reports.length, 0);
});
test('a changed URL, invalid owner, or disconnected element never decodes an obsolete image', async () => {
  for (const invalid of ['source', 'owner', 'disconnected']) {
    const f = fixture(); let current = true;
    const ready = prepareCaptureBackground(f.image, identity, () => current, f.report);
    if (invalid === 'source') f.image.source = '/replacement.png';
    if (invalid === 'owner') current = false;
    if (invalid === 'disconnected') f.image.isConnected = false;
    await ready; assert.deepEqual(f.events, []); assert.deepEqual(f.reports, []);
  }
});
test('missing attributes before attachment do not claim the later valid preparation', async () => {
  const f = fixture(); f.image.source = null;
  await prepareCaptureBackground(f.image, identity, () => true, f.report);
  assert.deepEqual(f.events, []);
  f.image.source = '/full-screen.png';
  const ready = prepareCaptureBackground(f.image, identity, () => true, f.report);
  await Promise.resolve(); f.release(); await ready;
  assert.deepEqual(f.reports, [{ ...identity, success: true }]);
});
test('a rejected native acknowledgement is not retried by a later load callback', async () => {
  const f = fixture(); let reports = 0;
  const report = async () => { reports++; throw Error('disconnected IPC'); };
  const first = prepareCaptureBackground(f.image, identity, () => true, report);
  await Promise.resolve(); f.release(); await assert.rejects(first, /disconnected IPC/);
  await assert.rejects(prepareCaptureBackground(f.image, identity, () => true, report), /disconnected IPC/);
  assert.equal(reports, 1);
});
test('a thumbnail or empty layout cannot satisfy full-resolution readiness', async () => {
  for (const invalid of ['width', 'height', 'layout', 'incomplete']) {
    const f = fixture();
    if (invalid === 'width') f.image.naturalWidth = 960;
    if (invalid === 'height') f.image.naturalHeight = 540;
    if (invalid === 'layout') f.image.getBoundingClientRect = () => ({ width: 0, height: 0 });
    if (invalid === 'incomplete') f.image.complete = false;
    const ready = prepareCaptureBackground(f.image, identity, () => true, f.report); f.release(); await ready;
    assert.deepEqual(f.reports, [{ ...identity, success: false }]);
  }
});
test('readiness permission is enabled only for the first-party Space', async () => {
  const directory = new URL('../src-tauri/capabilities/', import.meta.url);
  const granted = [];
  for (const file of await readdir(directory)) {
    if (!file.endsWith('.json')) continue;
    const capability = JSON.parse(await readFile(new URL(file, directory), 'utf8'));
    if (capability.permissions.includes('allow-capture-background-ready')) granted.push(capability.windows);
  }
  assert.deepEqual(granted, [['space']]);
});
