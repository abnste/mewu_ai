// node --experimental-vm-modules apps/desktop/src/drawing-tools.test.mjs
// Pure geometry, source-pixel grids, cache backpressure and completion ordering.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const load = async path => {
  const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(path, import.meta.url), 'utf8'), { mode: 'transform' }));
  await module.link(() => { throw new Error('Unexpected runtime dependency'); }); await module.evaluate(); return module.namespace;
};
const { constrainedEnd, drawingBounds, drawingOrder, validNextNumber } = await load('./components/drawing-geometry.ts');
const { finishDrawing, finishCapture } = await load('./capture-completion.ts');
const { mosaicGrid, mosaicBlockSize, MosaicPreviewCache } = await load('./mosaic-preview.ts');
const plain = value => JSON.parse(JSON.stringify(value));
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const checks = [];

const region = { x: 100, y: 100, width: 200, height: 120 }, start = { x: 130, y: 130 }, pointer = { x: 190, y: 150 };
assert.deepEqual(plain(constrainedEnd('line', start, pointer, true, region)), { x: 190, y: 130 });
assert.deepEqual(plain(constrainedEnd('arrow', start, { x: 180, y: 170 }, true, region)), { x: 175, y: 175 });
assert.deepEqual(plain(constrainedEnd('ellipse', { x: 280, y: 200 }, { x: 290, y: 219 }, true, region)), { x: 299, y: 219 });
assert.deepEqual(plain(constrainedEnd('rect', { x: 290, y: 210 }, { x: 300, y: 218 }, true, region)), { x: 300, y: 220 });
assert.deepEqual(plain(constrainedEnd('line', start, pointer, false, region)), pointer);
assert.deepEqual(plain(constrainedEnd('mosaic', start, pointer, true, region)), pointer);
// The same pointer is recomputed when Shift changes; no synthetic pointermove required.
const shifted = constrainedEnd('rect', start, pointer, true, region), released = constrainedEnd('rect', start, pointer, false, region);
assert.equal(shifted.x - start.x, shifted.y - start.y); assert.deepEqual(plain(released), pointer);
checks.push('Shift projects lines/arrows to nearest 45°, clips square/circle as one length, and restores the stationary pointer');

for (const value of ['0', '01', '99999', '-1', '1.2', '１', ' 1']) assert.equal(validNextNumber(value), undefined);
assert.equal(validNextNumber('9999'), 9999);
const number = { id: 'n', kind: 'number', points: [{ x: 20, y: 30 }], fontSize: 28, strokeWidth: 4, text: '1234' };
assert.deepEqual(plain(drawingBounds(number)), { x: 20, y: 30, width: 28, height: 28 });
const objects = [{ id: 'p', kind: 'pen' }, { id: 'm1', kind: 'mosaic' }, { id: 'h', kind: 'highlighter' }, { id: 'm2', kind: 'mosaic' }];
assert.deepEqual(drawingOrder(objects).map(value => value.id), ['m1', 'm2', 'p', 'h']);
assert.deepEqual(objects.map(value => value.id), ['p', 'm1', 'h', 'm2']);
checks.push('Number format and circle bounds match native; every mosaic precedes every vector without mutating history order');

const pixels = new Uint8ClampedArray(7 * 7 * 4);
for (let y = 0; y < 7; y++) for (let x = 0; x < 7; x++) pixels.set([x * 10, y * 10, x + y, 255], (y * 7 + x) * 4);
const grid = mosaicGrid(pixels, 7, 7, 6);
assert.equal(grid.columns, 2); assert.equal(grid.rows, 2);
assert.deepEqual([...grid.data], [25, 25, 5, 255, 60, 25, 8, 255, 25, 60, 8, 255, 60, 60, 12, 255]);
assert.deepEqual([...mosaicGrid(new Uint8ClampedArray([0, 0, 0, 0]), 1, 1, 6).data], [255, 255, 255, 255]);
assert.deepEqual([...mosaicGrid(new Uint8ClampedArray([100, 20, 200, 128]), 1, 1, 6).data], [177, 137, 227, 255]);
for (const [w, h, block] of [[0, 1, 6], [1, 1, 0], [1, 1, 6.5], [16385, 1, 6], [8192, 8192, 6]]) assert.throws(() => mosaicGrid(new Uint8ClampedArray(4), w, h, block));
assert.throws(() => mosaicGrid(new Uint8ClampedArray(3), 1, 1, 6));
assert.equal(mosaicBlockSize(), 12); assert.equal(mosaicBlockSize(1.5), 18); assert.equal(mosaicBlockSize(4), 40);
checks.push('Mosaic uses whole origin-aligned blocks, partial edges, exact native white-alpha/floor means and bounded valid image inputs');

const cache = new MosaicPreviewCache(16, 2), waits = [deferred(), deferred(), deferred()], started = [];
const pending = waits.map((wait, index) => cache.get('background', 6 + index, () => { started.push(index); return wait.promise; }));
assert.equal(cache.get('background', 6, () => { throw new Error('duplicate'); }), pending[0]);
await tick(); assert.deepEqual(started, [0, 1]);
waits[0].resolve({ dataUrl: 'small' }); await tick(); assert.deepEqual(started, [0, 1, 2]);
waits[1].resolve({ dataUrl: 'small' }); waits[2].resolve({ dataUrl: 'small' }); await Promise.all(pending);
let retries = 0; await assert.rejects(cache.get('failure', 6, async () => { retries++; throw new Error('decode failed'); }));
await cache.get('failure', 6, async () => { retries++; return { dataUrl: 'ok' }; }); assert.equal(retries, 2);
checks.push('Cache coalesces parallel requests, permits at most two loaders and retries failed previews');

const lru = new MosaicPreviewCache(2, 2, 8); let loads = 0;
const request = key => lru.get(key, 6, async () => { loads++; return { dataUrl: '12345' }; });
await request('a'); await request('b'); await request('b'); assert.equal(loads, 2);
await request('a'); assert.equal(loads, 3); // byte budget evicted a, even though item limit was not reached
const smallCache = new MosaicPreviewCache(1, 1), gate = deferred();
const first = smallCache.get('a', 6, () => gate.promise);
await assert.rejects(smallCache.get('b', 6, async () => ({ dataUrl: '' })), /繁忙/);
gate.resolve({ dataUrl: '' }); await first;
checks.push('Completed grids obey byte and entry LRU limits; saturated pending cache rejects rather than duplicating unbounded work');

for (const failure of ['save', 'copy']) {
  const events = [];
  const ok = await finishDrawing({ saveText: async () => { events.push('save'); return failure !== 'save'; }, copy: async () => { events.push('copy'); return failure !== 'copy'; }, current: () => true, close: () => events.push('close') });
  assert.equal(ok, false); assert.deepEqual(events, failure === 'save' ? ['save'] : ['save', 'copy']);
}
await assert.rejects(finishDrawing({ saveText: async () => { throw new Error('CAS'); }, copy: async () => assert.fail('copy after CAS failure'), current: () => true, close: () => assert.fail('closed after CAS failure') }), /CAS/);
checks.push('Text persistence or clipboard failure never copies/closes prematurely; CAS exceptions leave editing intact');

for (const at of ['save', 'copy']) {
  const wait = deferred(), events = []; let active = true;
  const done = finishDrawing({ saveText: async () => { events.push('save'); return at === 'save' ? wait.promise : true; }, copy: async () => { events.push('copy'); return wait.promise; }, current: () => active, close: () => events.push('close') });
  await tick(); active = false; wait.resolve(true); assert.equal(await done, false); assert.ok(!events.includes('close'));
  if (at === 'save') assert.deepEqual(events, ['save']);
}
const events = [], save = deferred(), copy = deferred();
const done = finishDrawing({ saveText: () => { events.push('save'); return save.promise; }, copy: () => { events.push('copy'); return copy.promise; }, current: () => true, close: () => events.push('close') });
assert.deepEqual(events, ['save']); save.resolve(true); await tick(); assert.deepEqual(events, ['save', 'copy']); copy.resolve(true); assert.equal(await done, true); assert.deepEqual(events, ['save', 'copy', 'close']);
checks.push('Late scene/unmount results never close a new scene; success strictly awaits text persistence then copy then close');
for (const failure of ['prepare', 'export', 'none']) {
  const events = [];
  const actions = { prepare: async () => { events.push('prepare'); if (failure === 'prepare') throw new Error(failure); }, current: () => true, export: async () => { events.push('export'); if (failure === 'export') throw new Error(failure); }, close: async () => { events.push('close-scene'); } };
  if (failure === 'none') assert.equal(await finishCapture(actions), true); else await assert.rejects(finishCapture(actions), new RegExp(failure));
  assert.deepEqual(events, failure === 'prepare' ? ['prepare'] : failure === 'export' ? ['prepare', 'export'] : ['prepare', 'export', 'close-scene']);
}
let current = true; const late = deferred(), closing = [];
const capture = finishCapture({ prepare: async () => {}, current: () => current, export: () => late.promise, close: async () => { closing.push('wrong-scene'); } });
await tick(); current = false; late.resolve(); assert.equal(await capture, false); assert.deepEqual(closing, []);
const keep = []; await finishCapture({ prepare: async () => {}, current: () => true, export: async () => { keep.push('export'); } }); assert.deepEqual(keep, ['export']);
assert.equal(await finishCapture({ prepare: async () => {}, current: () => true, export: async () => {}, close: async () => false }), false);
checks.push('The actual Enter export pipeline waits for queued writes and copy before close-scene; failures/late scenes never close, C/S have no close action');
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
