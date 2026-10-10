// node --experimental-vm-modules apps/desktop/src/window-snap.test.mjs
// Synthetic maps and pointer coordinates only: no OS window lookup or clipboard.
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
const source = async name => stripTypeScriptTypes(await readFile(new URL(name, import.meta.url), 'utf8'), { mode: 'transform' });
const module = new SourceTextModule(await source('./window-snap.ts')); await module.link(() => { throw Error('Unexpected import'); }); await module.evaluate();
const { validateSnapMap, snapTarget, snapResize, snapImagePoint, SnapSelectionGesture, SnapMapLoader } = module.namespace;
const checks = [], request = { sceneId: 'scene', backgroundId: 'bg', width: 1200, height: 800 };
const box = (x, y, width, height) => ({ x, y, width, height });
const node = (id, bounds, selectable = true, children = []) => ({ id, bounds, selectable, children });
const map = roots => ({ version: 1, backgroundId: 'bg', width: 1200, height: 800, roots });
{
  const value = map([node(1, box(0, 0, 1200, 40), false), node(2, box(100, 100, 700, 600), true, [node(3, box(150, 150, 400, 350), false, [node(4, box(200, 200, 100, 80))]), node(5, box(150, 150, 100, 100))]), node(6, box(0, 0, 1200, 800))]);
  const parsed = validateSnapMap(value, request); assert.ok(parsed);
  assert.equal(snapTarget(parsed, 10, 10), undefined); assert.equal(snapTarget(parsed, 200, 210).id, 4);
  assert.equal(snapTarget(parsed, 160, 170).id, 2); // nonselectable child occludes later sibling 5
  assert.equal(snapTarget(parsed, 800, 200).id, 6); assert.equal(snapTarget(parsed, 1200, 200), undefined);
  assert.equal(snapTarget(parsed, 300, 220).id, 2); assert.equal(snapTarget(parsed, NaN, 200), undefined);
  value.roots[1].bounds.x = 999; assert.equal(parsed.roots[1].bounds.x, 100);
  checks.push('First Z branch owns the half-open point; blockers never expose a lower root/sibling and unselectable descendants fall back to the nearest selectable ancestor');
}
{
  const valid = map([node(1, box(0, 0, 1200, 800))]);
  for (const bad of [{ ...valid, version: 2 }, { ...valid, backgroundId: 'old' }, { ...valid, width: 1000 }, { ...valid, roots: [node(-1, box(0, 0, 10, 10))] }, { ...valid, roots: [node(1, box(0, 0, 10.5, 10))] }, { ...valid, roots: [node(1, box(0, 0, 10, 10), false, [node(2, box(0, 0, 5, 5))])] }, { ...valid, roots: [node(1, box(0, 0, 10, 10), true, [node(2, box(9, 0, 5, 5))])] }, { ...valid, roots: [node(1, box(0, 0, 10, 10)), node(1, box(5, 5, 10, 10))] }, { ...valid, extra: true }]) assert.equal(validateSnapMap(bad, request), undefined);
  assert.equal(validateSnapMap(map(Array.from({ length: 257 }, (_, i) => node(i, box(0, 0, 1, 1)))), request), undefined);
  let branch = node(13, box(0, 0, 1, 1)); for (let i = 12; i > 0; i--) branch = node(i, box(0, 0, 1, 1), true, [branch]);
  assert.equal(validateSnapMap(map([branch]), request), undefined); branch = branch.children[0]; assert.ok(validateSnapMap(map([branch]), request));
  const children = Array.from({ length: 2047 }, (_, i) => node(i + 1, box(0, 0, 1, 1))); assert.ok(validateSnapMap(map([node(0, box(0, 0, 1200, 800), true, children)]), request));
  children.push(node(2048, box(0, 0, 1, 1))); assert.equal(validateSnapMap(map([node(0, box(0, 0, 1200, 800), true, children)]), request), undefined);
  const hugeIdentity = '界'.repeat(90_000); assert.equal(validateSnapMap({ ...valid, backgroundId: hugeIdentity }, { ...request, backgroundId: hugeIdentity }), undefined);
  assert.equal(validateSnapMap({ ...valid, width: 16384, height: 16384 }, { ...request, width: 16384, height: 16384 }), undefined);
  checks.push('Typed map validation enforces identity, unsigned integers, parent clipping, unique IDs, root blockers, depth 12, 256 roots, 2048 nodes, 256KiB UTF-8 and image dimensions');
}
{
  const image = box(100, 75, 600, 400);
  assert.deepEqual(snapImagePoint(200, 125, image, .5), { x: 200, y: 100 });
  assert.equal(snapImagePoint(99, 125, image, .5), undefined); assert.equal(snapImagePoint(700, 125, image, .5), undefined); assert.equal(snapImagePoint(200, 475, image, .5), undefined);
  const candidate = box(110, 80, 300, 200), gesture = new SnapSelectionGesture({ x: 120, y: 90 }, candidate);
  assert.deepEqual(gesture.move({ x: 124, y: 94 }), candidate); const dragged = gesture.move({ x: 124.01, y: 90 }); assert.ok(Math.abs(dragged.width - 4.01) < 1e-10); assert.equal(dragged.height, 0);
  assert.equal(gesture.isAutomatic, false); assert.deepEqual(gesture.move({ x: 120, y: 90 }), box(120, 90, 0, 0));
  const edge = new SnapSelectionGesture({ x: 101, y: 100 }, candidate); edge.move({ x: 100, y: 100 }, false, { x: 80, y: 100 }); assert.equal(edge.isAutomatic, false);
  const shift = new SnapSelectionGesture({ x: 120, y: 90 }, candidate); shift.forceManual(); assert.equal(shift.isAutomatic, false);
  checks.push('Contain uses CSS coordinates without screen-origin/DPI multiplication; strict >4px per axis commits to manual mode even when dragged outside the clamped image');
}
{
  const value = box(100, 100, 200, 150), target = box(104, 105, 200, 150);
  const expected = { nw: box(104, 105, 196, 145), n: box(100, 105, 200, 145), ne: box(100, 105, 204, 145), w: box(104, 100, 196, 150), e: box(100, 100, 204, 150), sw: box(104, 100, 196, 155), s: box(100, 100, 200, 155), se: box(100, 100, 204, 155) };
  for (const [handle, wanted] of Object.entries(expected)) assert.deepEqual(snapResize(value, handle, target, 1, 1200, 800, 8), wanted);
  assert.deepEqual(snapResize(value, 'e', box(308, 20, 100, 100), 1, 1200, 800, 8), box(100, 100, 208, 150));
  assert.deepEqual(snapResize(value, 'e', box(310, 20, 100, 100), 1, 1200, 800, 8), value);
  assert.equal(snapResize(value, 'e', box(315, 20, 100, 100), .5, 1200, 800, 16).width, 215);
  assert.equal(snapResize(value, 'e', box(295, 20, 10, 100), 1, 1200, 800, 8).width, 195); // tie chooses first
  assert.deepEqual(snapResize(value, 'w', box(294, 20, 100, 100), .04, 1200, 800, 8), value); // cannot cross opposite edge/minimum
  assert.deepEqual(snapResize(value, 'w', box(-4, 20, 100, 100), .05, 1200, 800, 8), box(96, 100, 204, 150));
  assert.deepEqual(snapResize(value, 'e', undefined, 1, 1200, 800, 8), value);
  checks.push('All eight resize handles snap only moved edges within 9 CSS px, preserve opposite edges, prefer left/top on ties and reject a smaller-than-minimum rectangle');
}
{
  const jobs = [], published = []; let flight = 0, maximum = 0;
  const loader = new SnapMapLoader(value => new Promise((resolve, reject) => { flight++; maximum = Math.max(maximum, flight); jobs.push({ value, resolve: result => { flight--; resolve(result); }, reject: error => { flight--; reject(error); } }); }), value => published.push(value));
  const tick = () => new Promise(resolve => setImmediate(resolve));
  loader.request(request); loader.request(request); assert.equal(jobs.length, 1);
  loader.request({ ...request, sceneId: 'B', backgroundId: 'b' }); loader.request({ ...request, sceneId: 'C', backgroundId: 'c' }); assert.equal(jobs.length, 1);
  jobs[0].resolve(map([node(1, box(0, 0, 20, 20))])); await tick(); assert.equal(jobs.length, 2); assert.equal(jobs[1].value.sceneId, 'C'); assert.equal(published.filter(Boolean).length, 0);
  loader.cancel(); loader.request({ ...request, sceneId: 'D', backgroundId: 'd' }); assert.equal(jobs.length, 2); jobs[1].reject(Error('stale')); await tick(); assert.equal(jobs.length, 3);
  jobs[2].resolve({ ...map([node(2, box(0, 0, 30, 30))]), backgroundId: 'd' }); await tick(); assert.equal(published.at(-1).backgroundId, 'd'); assert.equal(maximum, 1);
  loader.request({ ...request, sceneId: 'D', backgroundId: 'd' }); assert.equal(jobs.length, 3);
  loader.request({ ...request, sceneId: 'E' }); loader.dispose(); jobs[3].resolve(map([])); await tick(); assert.equal(published.at(-1), undefined);
  const failed = [], silent = new SnapMapLoader(async () => { throw Error('unavailable'); }, value => failed.push(value)); silent.request(request); await tick(); assert.deepEqual(failed, [undefined, undefined]); silent.request(request); await tick(); assert.equal(failed.length, 2); silent.dispose();
  checks.push('One actual request plus only the latest identity survives scene changes; canceled/old replies cannot publish, unavailable maps stay silent and unmount rejects late completion');
}
{
  let native = false; const calls = [];
  const core = new SyntheticModule(['invoke', 'isTauri'], function () { this.setExport('isTauri', () => native); this.setExport('invoke', async (command, args) => { calls.push({ command, args }); return null; }); });
  const bridge = new SourceTextModule(await source('./window-snap-bridge.ts')); await bridge.link(() => core); await bridge.evaluate();
  assert.equal(await bridge.namespace.getWindowSnapMap(request), null); assert.equal(calls.length, 0);
  native = true; await bridge.namespace.getWindowSnapMap(request); assert.deepEqual(calls, [{ command: 'get_window_snap_map', args: { sceneId: 'scene', backgroundId: 'bg' } }]);
  checks.push('Browser performs no desktop discovery; native IPC carries only the exact scene/background ownership request');
}
{
  const jobs = [], published = []; let flight = 0, maximum = 0;
  const core = new SyntheticModule(['invoke', 'isTauri'], function () { this.setExport('isTauri', () => true); this.setExport('invoke', (command, args) => new Promise(resolve => { flight++; maximum = Math.max(maximum, flight); jobs.push({ args, resolve: value => { flight--; resolve(value); } }); })); });
  const bridge = new SourceTextModule(await source('./window-snap-bridge.ts')); await bridge.link(() => core); await bridge.evaluate();
  const make = () => new SnapMapLoader(bridge.namespace.getWindowSnapMap, value => { if (value) published.push(value.backgroundId); });
  const first = make(); first.request(request); first.dispose();
  const second = make(); second.request({ ...request, sceneId: 'second', backgroundId: 'second-bg' }); second.dispose();
  const third = make(); third.request({ ...request, sceneId: 'third', backgroundId: 'third-bg' });
  assert.equal(jobs.length, 1); jobs[0].resolve(map([])); await new Promise(resolve => setImmediate(resolve));
  assert.equal(jobs.length, 2); assert.equal(jobs[1].args.sceneId, 'third'); assert.deepEqual(published, []);
  jobs[1].resolve({ ...map([]), backgroundId: 'third-bg' }); await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(published, ['third-bg']); assert.equal(maximum, 1); third.dispose();
  checks.push('Keyed Canvas destruction/recreation cannot bypass the module IPC slot; only the latest queued scene starts after the old native invocation settles');
}
{
  const directory = new URL('../src-tauri/capabilities/', import.meta.url);
  const capabilities = await Promise.all((await readdir(directory)).filter(name => name.endsWith('.json')).map(async name => JSON.parse(await readFile(new URL(name, directory), 'utf8'))));
  const owners = capabilities.filter(value => value.permissions.includes('allow-get-window-snap-map'));
  assert.equal(owners.length, 1); assert.equal(owners[0].identifier, 'space'); assert.deepEqual(owners[0].windows, ['space']);
  const config = JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  assert.ok(config.app.security.capabilities.includes(owners[0].identifier));
  checks.push('Only the enabled first-party space capability can read captured window metadata; settings, pins and control windows receive no map permission');
}
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
