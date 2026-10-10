// node --experimental-vm-modules apps/desktop/src/eraser-tools.test.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./components/drawing-eraser.ts', import.meta.url), 'utf8'), { mode: 'transform' }));
await module.link(() => { throw new Error('Unexpected runtime dependency'); }); await module.evaluate();
const { eraserHit, ObjectEraser } = module.namespace;
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; };
const checks = [];

{
  const removed = [], objects = ['lower', 'upper']; let eraser;
  eraser = new ObjectEraser({ x: 100, y: 100 }, { current: () => true, hit: point => { eraser.move(point); return objects.at(-1); }, remove: async id => { removed.push(id); objects.pop(); return true; }, stopped() {} });
  eraser.move({ x: 100, y: 100 }); // synchronous capture resync before initial hit
  eraser.start(); eraser.start(); await tick(); eraser.move({ x: 100, y: 100 }); await tick();
  assert.deepEqual(removed, ['upper']); assert.deepEqual(objects, ['lower']); eraser.stop();
  checks.push('Capture and DOM resync at the same point cannot penetrate through a removed upper object');
}
{
  const hit = [];
  const eraser = new ObjectEraser({ x: 0, y: 0 }, { current: () => true, hit: point => { hit.push([point.x, point.y]); }, remove: async () => assert.fail('No hit'), stopped() {} });
  eraser.start(); eraser.move({ x: 11.99, y: 0 }); eraser.move({ x: 7.2, y: 9.6 }); eraser.move({ x: 7.2, y: 9.6 }); eraser.move({ x: 19.2, y: 9.6 });
  eraser.move({ x: NaN, y: 100 });
  assert.deepEqual(hit, [[0, 0], [7.2, 9.6], [19.2, 9.6]]); eraser.stop();
  checks.push('The 12 CSS-pixel distance is Euclidean, inclusive, and advances even when no object is hit');
}
{
  const first = deferred(), second = deferred(), calls = [], hits = []; let revision = 3;
  const eraser = new ObjectEraser({ x: 0, y: 0 }, { current: () => true, hit: point => { hits.push(point.x); return `object-${point.x}`; }, remove: async id => { calls.push([id, revision]); await (calls.length === 1 ? first.promise : second.promise); revision++; return true; }, stopped() {} });
  eraser.start(); eraser.move({ x: 12, y: 0 }); eraser.move({ x: 24, y: 0 }); eraser.move({ x: 36, y: 0 });
  assert.deepEqual(hits, [0]); assert.equal(calls.length, 1);
  first.resolve(); await tick(); assert.deepEqual(hits, [0, 36]); assert.deepEqual(calls, [['object-0', 3], ['object-36', 4]]);
  second.resolve(); await tick(); assert.equal(calls.length, 2); eraser.stop();
  checks.push('Only one deletion is in flight; pending motion is bounded to its latest real sample and uses the updated revision');
}
{
  const wait = deferred(), calls = []; let cleanups = 0;
  const eraser = new ObjectEraser({ x: 0, y: 0 }, { current: () => true, hit: point => String(point.x), remove: async id => { calls.push(id); await wait.promise; return true; }, stopped() { cleanups++; } });
  eraser.start(); eraser.move({ x: 15, y: 0 }); eraser.stop(); eraser.stop(); wait.resolve(); await tick(); eraser.move({ x: 30, y: 0 });
  assert.deepEqual(calls, ['0']); assert.equal(cleanups, 1);
  checks.push('Release/cancel drops queued movement and cleans up once without claiming to roll back an already submitted deletion');
}
{
  const wait = deferred(), calls = []; let current = true, cleanups = 0;
  const eraser = new ObjectEraser({ x: 0, y: 0 }, { current: () => current, hit: point => String(point.x), remove: async id => { calls.push(id); await wait.promise; return true; }, stopped() { cleanups++; } });
  eraser.start(); eraser.move({ x: 15, y: 0 }); current = false; wait.resolve(); await tick();
  assert.deepEqual(calls, ['0']); assert.equal(cleanups, 1);
  checks.push('Scene/source/authorization generation changes fence subsequent queued deletions after an asynchronous response');
}
{
  let objects = ['one'], failed = true, cleanups = 0; const calls = [];
  const hooks = { current: () => true, hit: () => objects[0], remove: async id => { calls.push(id); if (failed) return false; objects = []; return true; }, stopped() { cleanups++; } };
  const first = new ObjectEraser({ x: 0, y: 0 }, hooks); first.start(); first.move({ x: 20, y: 0 }); await tick(); assert.deepEqual(objects, ['one']); assert.equal(cleanups, 1);
  failed = false; const retry = new ObjectEraser({ x: 0, y: 0 }, hooks); retry.start(); await tick(); retry.stop(); assert.deepEqual(objects, []); assert.deepEqual(calls, ['one', 'one']);
  checks.push('Failed persistence stops the stroke, preserves its object, and permits a fresh gesture to retry');
}
{
  const children = new Set(), svg = { getBoundingClientRect: () => ({ left: 10, top: 20, right: 210, bottom: 120 }), contains: value => children.has(value) };
  const element = (id, definitions = false) => { const group = { getAttribute: () => id }; const child = { closest: query => query === '[data-drawing-id]' ? group : definitions ? {} : null }; children.add(group); children.add(child); return child; };
  const top = element('vector-top'), lower = element('mosaic-lower'), otherRegion = element('foreign'), definition = element('vector-top', true), toolbar = {};
  const allowed = new Set(['vector-top', 'mosaic-lower']), point = { x: 50, y: 50 };
  assert.equal(eraserHit(svg, point, allowed, [top, lower, svg]), 'vector-top');
  assert.equal(eraserHit(svg, point, allowed, [lower, top, svg]), 'mosaic-lower'); // trusts actual browser paint order
  assert.equal(eraserHit(svg, point, allowed, [toolbar, top, svg]), undefined);
  assert.equal(eraserHit(svg, point, allowed, [otherRegion, lower, svg]), 'mosaic-lower');
  assert.equal(eraserHit(svg, point, allowed, [definition, svg]), undefined);
  assert.equal(eraserHit(svg, point, allowed, [svg, top]), undefined);
  for (const outside of [{ x: 9, y: 50 }, { x: 210, y: 50 }, { x: 50, y: 120 }, { x: NaN, y: 50 }]) assert.equal(eraserHit(svg, outside, allowed, [top, svg]), undefined);
  checks.push('Hit testing respects browser paint order, SVG membership and clipping bounds; toolbar, source image and foreign IDs cannot be erased');
}
for (const check of checks) console.log(`PASS ${check}`);
console.log(`${checks.length} eraser checks passed`);
