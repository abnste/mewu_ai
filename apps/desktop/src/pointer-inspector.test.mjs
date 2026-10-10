// node --experimental-vm-modules apps/desktop/src/pointer-inspector.test.mjs
// Pure geometry and synthetic promises only: no browser, native IPC or pixels read.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./pointer-inspector.ts', import.meta.url), 'utf8'), { mode: 'transform' }));
await module.link(() => { throw Error('Unexpected runtime dependency'); }); await module.evaluate();
const { PointerInspector, PointerSampleCanceledError, pointerImagePoint, pointerPopupPosition, pointerSelectionDimensions, pointerInspectorSize } = module.namespace;
const plain = value => JSON.parse(JSON.stringify(value));
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const request = (x = 10, sceneId = 'scene', backgroundId = 'background') => ({ sceneId, backgroundId, x, y: 20 });
const sample = value => ({ ...value, globalX: value.x - 1920, globalY: value.y, hex: '#1234AB', dataUrl: 'data:image/png;base64,AAAA' });
const checks = [];

const box = { x: 100, y: 40, width: 800, height: 600 }, image = { width: 1600, height: 900 };
assert.equal(pointerImagePoint(500, 100, box, image), undefined); // top letterbox is 75 CSS px
assert.equal(pointerImagePoint(500, 566, box, image), undefined);
assert.deepEqual(plain(pointerImagePoint(100, 115, box, image)), { x: 0, y: 0 });
assert.equal(pointerImagePoint(900, 564, box, image), undefined);
assert.equal(pointerImagePoint(899, 565, box, image), undefined);
assert.deepEqual(plain(pointerImagePoint(899.99, 564.99, box, image)), { x: 1599, y: 899 });
assert.deepEqual(plain(pointerImagePoint(100.26, 115.26, box, image)), { x: 1, y: 1 });
assert.deepEqual(plain(pointerImagePoint(500, 340, box, image)), { x: 800, y: 450 });
const portrait = { width: 200, height: 1000 };
assert.equal(pointerImagePoint(300, 300, box, portrait), undefined);
assert.deepEqual(plain(pointerImagePoint(500, 340, box, portrait)), { x: 100, y: 500 });
for (const [x, y, b, i] of [[NaN, 0, box, image], [0, Infinity, box, image], [0, 0, { ...box, width: 0 }, image], [0, 0, box, { width: 1.5, height: 10 }], [0, 0, { ...box, x: Infinity }, image]]) assert.equal(pointerImagePoint(x, y, b, i), undefined);
checks.push('Contain mapping rejects letterboxes/non-finite geometry, rounds source pixels and clamps all image edges');

assert.deepEqual(plain(pointerPopupPosition(50, 50, 800, 600)), { left: 66, top: 66 });
assert.deepEqual(plain(pointerPopupPosition(790, 590, 800, 600)), { left: 684, top: 449 });
assert.deepEqual(plain(pointerPopupPosition(-100, -100, 800, 600)), { left: 4, top: 4 });
assert.deepEqual(plain(pointerPopupPosition(10, 10, 80, 100)), { left: 4, top: 4 });
assert.equal(pointerPopupPosition(NaN, 1, 800, 600), undefined);
assert.deepEqual(plain(pointerPopupPosition(790, 590, 800, 600, { width: 100, height: 150 })), { left: 674, top: 424 });
checks.push('90×125 popup uses 16px gap, flips at viewport edges and preserves 4px margin');

{
  const image = { x: 100, y: 50, scale: 1.25 }, old = { width: 640, height: 300 };
  assert.equal(pointerSelectionDimensions(undefined, image), undefined);
  assert.equal(pointerSelectionDimensions(undefined, image, old), '640 × 300');
  // The left edge rounds up: round(width / scale) alone would incorrectly show 2.
  assert.equal(pointerSelectionDimensions({ x: 100.75, y: 50.75, width: 2, height: 2 }, image, old), '1 × 1');
  assert.equal(pointerSelectionDimensions({ x: 100, y: 50, width: 0, height: 0 }, image, old), undefined);
  assert.equal(pointerSelectionDimensions(undefined, image, { ...old, width: 641 }), '641 × 300');
  assert.equal(pointerSelectionDimensions(undefined, image, { ...old, imageOverride: { width: 300, height: 1200 } }), '300 × 1200');
  assert.equal(pointerSelectionDimensions(undefined, image, undefined), undefined); // deleted selection leaves no blank size row
  assert.equal(pointerSelectionDimensions({ x: 1, y: 1, width: 2, height: 2 }, { ...image, scale: 0 }, old), undefined);
  assert.equal(pointerSelectionDimensions(undefined, image, { width: NaN, height: 2 }), undefined);
  const small = pointerInspectorSize(false), full = pointerInspectorSize(true);
  assert.deepEqual(plain(small), { width: 90, height: 126 }); assert.deepEqual(plain(full), { width: 90, height: 144 });
  const a = pointerPopupPosition(790, 590, 800, 600, small), b = pointerPopupPosition(790, 590, 800, 600, full);
  assert.equal(b.top, a.top - 18); assert.equal(b.left, a.left); assert.ok(b.top + full.height <= 596);
  checks.push('Footer follows source-rounded live selection, committed resize and override pixels; no selection removes the row and edge placement reserves its full extra height');
}

function fixture() {
  const started = [], outputs = [], errors = [], waits = [];
  const gate = new PointerInspector(value => { started.push(plain(value)); const wait = deferred(); waits.push(wait); return wait.promise; }, value => outputs.push(value && plain(value)), error => errors.push(error));
  return { gate, started, outputs, errors, waits };
}
{
  const f = fixture(); f.gate.request(request(1)); f.gate.request(request(2)); f.gate.request(request(3));
  assert.equal(f.started.length, 1); f.waits[0].resolve(sample(request(1))); await tick();
  assert.deepEqual(f.started.map(v => v.x), [1, 3]); assert.equal(f.outputs.at(-1).x, 1);
  f.waits[1].resolve(sample(request(3))); await tick(); assert.equal(f.outputs.at(-1).x, 3);
  f.gate.request(request(3)); assert.equal(f.started.length, 2);
  assert.equal((await f.gate.sampleExact(request(3))).x, 3); assert.equal(f.started.length, 2);
  checks.push('Only one fetch runs and the waiting position is replaced; same-source completions preview during movement and the current exact sample is reused');
}
{
  const f = fixture(); f.gate.request(request(1));
  f.waits[0].resolve(sample(request(1))); await tick();
  const clears = f.outputs.filter(value => value === undefined).length;
  for (let x = 2; x <= 6; x++) {
    f.gate.request(request(x)); f.gate.request(request(x + 1));
    assert.ok(f.outputs.at(-1));
    f.waits.at(-1).resolve(sample(f.started.at(-1))); await tick();
    assert.equal(f.outputs.filter(value => value === undefined).length, clears);
    assert.equal(f.outputs.at(-1).x, f.started.at(-2).x);
  }
  assert.equal(f.errors.length, 0);
  f.waits.at(-1).resolve(sample(f.started.at(-1))); await tick();
  assert.equal(f.outputs.at(-1).x, 7);
  checks.push('Continuous movement keeps the preview visible and publishes every completed sample without waiting for the pointer to stop');
}
{
  const f = fixture(); f.gate.request(request(1)); f.gate.request(request(2));
  let copied = false;
  const exact = f.gate.sampleExact(request(2)).then(value => { copied = true; return value; });
  f.waits[0].resolve(sample(request(1))); await tick();
  assert.equal(f.outputs.at(-1).x, 1); assert.equal(copied, false);
  f.waits[1].resolve(sample(request(2))); assert.equal((await exact).x, 2); await tick();
  const clears = f.outputs.filter(value => value === undefined).length;
  f.gate.request(request(3)); f.gate.request(request(4));
  f.waits[2].reject(Error('superseded failure')); await tick();
  assert.equal(f.outputs.at(-1).x, 2); assert.equal(f.outputs.filter(value => value === undefined).length, clears);
  assert.deepEqual(f.errors, []);
  f.waits[3].resolve(sample(request(4))); await tick(); assert.equal(f.outputs.at(-1).x, 4);
  checks.push('A lagging preview never resolves exact-copy; superseded errors do not blank a valid preview or report obsolete failures');
}
{
  for (const invalidate of [gate => gate.cancel(), gate => gate.request(request(2, 'other-scene')), gate => gate.request(request(2, 'scene', 'other-background'))]) {
    const f = fixture(); f.gate.request(request(1)); invalidate(f.gate); f.gate.request(request(2));
    f.waits[0].resolve(sample(request(1))); await tick();
    assert.ok(f.outputs.every(value => value === undefined));
    assert.equal(f.started.length, 2);
    f.waits[1].resolve(sample(request(2))); await tick(); assert.equal(f.outputs.at(-1).x, 2);
  }
  checks.push('Hide or source changes invalidate old previews even when the pointer returns to the same source before a late completion');
}
{
  const f = fixture(); const a = f.gate.sampleExact(request(1)); const rejected = assert.rejects(a, PointerSampleCanceledError);
  f.gate.request(request(1, 'other-scene')); await rejected;
  f.waits[0].reject(Error('old error')); await tick(); assert.deepEqual(f.errors, []);
  f.gate.request(request(1, 'other-scene', 'new-background'));
  f.waits[1].resolve(sample(request(1, 'other-scene'))); await tick();
  assert.ok(f.outputs.every(v => v === undefined));
  const exact = f.gate.sampleExact(request(1, 'other-scene', 'new-background')); assert.equal(f.started.length, 3);
  f.waits[2].resolve(sample(request(1, 'other-scene', 'new-background')));
  assert.equal((await exact).backgroundId, 'new-background'); await tick();
  checks.push('Exact-copy waits for matching scene/background/coordinates; changing identity rejects old waiters and suppresses old errors');
}
{
  const f = fixture(); const exact = f.gate.sampleExact(request()); const rejected = assert.rejects(exact, /synthetic failure/);
  f.waits[0].reject(Error('synthetic failure')); await rejected; await tick();
  assert.equal(f.outputs.at(-1), undefined); assert.deepEqual(f.errors, ['synthetic failure']);
  f.gate.request(request()); assert.equal(f.started.length, 2);
  f.waits[1].resolve(sample(request())); await tick(); assert.equal(f.outputs.at(-1).hex, '#1234AB');
  checks.push('Current failure clears sample and exact-copy rejects; a later explicit request can retry');
}
{
  for (const mutate of [v => ({ ...v, x: v.x + 1 }), v => ({ ...v, globalX: NaN }), v => ({ ...v, hex: '#12345678' }), v => ({ ...v, dataUrl: 'https://example.invalid/image.png' })]) {
    const f = fixture(); const exact = f.gate.sampleExact(request()); const rejected = assert.rejects(exact, /不一致/);
    f.waits[0].resolve(mutate(sample(request()))); await rejected; await tick();
    assert.equal(f.outputs.at(-1), undefined); assert.equal(f.errors.length, 1);
  }
  checks.push('Malformed or mismatched native response never becomes a displayed or copied color');
}
{
  const f = fixture(); const exact = f.gate.sampleExact(request()); const rejected = assert.rejects(exact, PointerSampleCanceledError);
  f.gate.cancel(); await rejected; f.gate.request(request(2)); assert.equal(f.started.length, 1);
  f.waits[0].resolve(sample(request())); await tick(); assert.equal(f.started.length, 2);
  const second = f.gate.sampleExact(request(2)); const disposed = assert.rejects(second, PointerSampleCanceledError);
  f.gate.dispose(); await disposed; f.waits[1].resolve(sample(request(2))); await tick();
  assert.ok(f.outputs.every(v => v === undefined)); assert.equal(f.errors.length, 0);
  f.gate.request(request(3)); assert.equal(f.started.length, 2); await assert.rejects(f.gate.sampleExact(request(3)), PointerSampleCanceledError);
  checks.push('Cancel/dispose invalidate generations synchronously without freeing an in-flight slot or publishing a late response');
}
{
  const f = fixture(); const a = f.gate.sampleExact(request()); const b = f.gate.sampleExact(request());
  assert.equal(a, b); assert.equal(f.started.length, 1); f.waits[0].resolve(sample(request())); await a;
  const calls = [], outputs = [];
  const sync = new PointerInspector(() => { calls.push(1); throw Error('synchronous failure'); }, v => outputs.push(v));
  await assert.rejects(sync.sampleExact(request()), /synchronous failure/); assert.equal(outputs.at(-1), undefined);
  checks.push('Repeated exact reads share one promise; synchronous adapter failures release the request slot');
}
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
