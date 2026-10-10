// node --experimental-vm-modules apps/desktop/src/eraser-tools.test.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./components/drawing-eraser.ts', import.meta.url), 'utf8'), { mode: 'transform' }));
await module.link(() => { throw new Error('Unexpected runtime dependency'); }); await module.evaluate();
const { eraserHit, eraserStrokeHits, eraserCursor, clipEraserSweep, ObjectEraser } = module.namespace;
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
  assert.deepEqual(hit, [[0, 0], [11.99,0], [7.2, 9.6], [19.2, 9.6]]); eraser.stop();
  checks.push('Short real moves are sampled immediately; identical/invalid capture resync does not re-hit');
}
{
  const first = deferred(), second = deferred(), calls = [], hits = []; let revision = 3;
  const eraser = new ObjectEraser({ x: 0, y: 0 }, { current: () => true, hit: point => { hits.push(point.x); return `object-${point.x}`; }, remove: async id => { calls.push([id, revision]); await (calls.length === 1 ? first.promise : second.promise); revision++; return true; }, stopped() {} });
  eraser.start(); eraser.move({ x: 12, y: 0 }); eraser.move({ x: 24, y: 0 }); eraser.move({ x: 36, y: 0 });
  assert.deepEqual(hits, [0,12,24,36]); assert.equal(calls.length, 1);
  first.resolve(); await tick(); assert.deepEqual(calls, [['object-0', 3], ['object-12', 4]]);
  second.resolve(); await tick(); assert.deepEqual(calls,[['object-0',3],['object-12',4],['object-24',5],['object-36',6]]); eraser.stop();
  checks.push('Unique hit IDs are collected during motion; serial IPC uses each updated revision without losing intermediate ink');
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

const dot=(id,x,y,strokeWidth=2)=>({id,kind:'pen',strokeWidth,color:'#ff0000',points:[{x,y}]});
{
  const drawings=[dot('touch',0,0),dot('outside',0,3.01),{...dot('image',0,0),kind:'rich'}];
  assert.deepEqual([...eraserStrokeHits(drawings,{x:0,y:3},{x:0,y:3},4)],['touch','outside']);
  assert.deepEqual([...eraserStrokeHits([dot('edge',0,0)],{x:0,y:3.01},{x:0,y:3.01},4)],[]);
  assert.deepEqual([...eraserStrokeHits([dot('edge',0,0)],{x:0,y:3},{x:0,y:3},4)],['edge']);
  const crossing={...dot('crossing',0,0),points:[{x:500,y:-10},{x:500,y:10}]};
  assert.deepEqual([...eraserStrokeHits([dot('middle',250,0),crossing],{x:0,y:0},{x:1000,y:0},4)],['middle','crossing']);
  assert.deepEqual([...eraserStrokeHits([dot('miss',250,4)],{x:0,y:0},{x:1000,y:0},4)],[]);
  assert.deepEqual([...eraserStrokeHits([dot('nan',0,0)],{x:NaN,y:0},{x:1,y:1},4)],[]);
  assert.match(eraserCursor(18),/^url\("data:image\/svg\+xml,/);
  assert.deepEqual(JSON.parse(JSON.stringify(clipEraserSweep({x:-20,y:10},{x:20,y:10},{x:0,y:0,width:100,height:100}))),[{x:0,y:10},{x:20,y:10}]);
  assert.equal(clipEraserSweep({x:-20,y:10},{x:-1,y:10},{x:0,y:0,width:100,height:100}),undefined);
  console.log('PASS Initial disks, exact tangency, single dots, fast sweeps, crossing strokes, and image exclusion');
}
{
  const gate=deferred(),calls=[];let cleanups=0,complete=false;
  const eraser=new ObjectEraser({x:0,y:0},{current:()=>true,hit:(point,previous)=>point.x===0?['a','a']:['a','b','c'],remove:async id=>{calls.push(id);if(id==='a')await gate.promise;return true;},stopped:()=>cleanups++});
  eraser.start();eraser.move({x:1,y:0});eraser.finish();eraser.finished.then(()=>complete=true);
  await tick();assert.equal(complete,false);assert.deepEqual(calls,['a']);
  gate.resolve();await eraser.finished;assert.deepEqual(calls,['a','b','c']);assert.equal(cleanups,1);
  console.log('PASS Pointer release drains accepted unique hits; completion waits for every native receipt');
}
