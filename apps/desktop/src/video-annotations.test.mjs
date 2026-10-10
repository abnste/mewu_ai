// SPDX-License-Identifier: MPL-2.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';
import { parse } from '@babel/parser';

const toUrl = code => `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`;
const moduleUrls = new Map();
async function load(name) {
  if (moduleUrls.has(name)) return moduleUrls.get(name);
  let code = stripTypeScriptTypes(await readFile(new URL(`./${name}.ts`, import.meta.url), 'utf8'), { mode: 'transform' });
  for (const dependency of [...code.matchAll(/from ['"]\.\/([^'"]+)['"]/g)].map(value => value[1])) code = code.replaceAll(`'./${dependency}'`, JSON.stringify(await load(dependency))).replaceAll(`"./${dependency}"`, JSON.stringify(await load(dependency)));
  const result = toUrl(code); moduleUrls.set(name, result); return result;
}
const annotations = await import(await load('video-annotations'));
const preview = await import(await load('video-annotation-preview'));
const { VideoAnnotationReader } = await import(await load('video-annotation-reader'));
const { VideoAnnotationPlaybackHold } = await import(await load('video-annotation-playback'));
const { VideoPlayback } = await import(await load('video-media'));
const { TrimCanceled } = await import(await load('video-trim'));
const { assertVideoTextDraftsSaved } = await import(await load('video-manual-edit'));
const turn = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const uuid = n => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
const hash = n => String(n).repeat(64);
const T = 10_000_000;
function fixture() {
  const object = { id: uuid(4), interval: { startTicks: 2 * T, endTicks: 3 * T }, primitive: { kind: 'rect', bounds: { x: 32, y: 10, width: 80, height: 40 }, style: { color: '#ff0000', strokeWidth: 2, opacity: 1 } }, origin: { runId: uuid(5), userMessageId: uuid(6), toolEventId: uuid(7), groupId: uuid(8), targetHandle: 'v1', manifestSha256: hash(1), createdSha256: hash(2) } };
  const item = { id: uuid(2), asset: { id: uuid(3), kind: 'video', name: 'synthetic.mp4', path: '/synthetic.mp4' }, x: .1, y: .2, width: .4, height: .3, videoEdit: { version: 1, sourceId: uuid(3), sourceDurationTicks: 10 * T, revision: 3, range: { startTicks: 2.5 * T, endTicks: 8 * T }, undo: [], redo: [] }, videoAnnotations: { version: 1, clock: 'sourcePlaybackTicks', sourceId: uuid(3), sourceDurationTicks: 10 * T, sourceWidth: 320, sourceHeight: 180, revision: 4, objects: [object], undo: [], redo: [] } };
  const scene = { id: uuid(1), closed: false, frozen: false, items: [item] };
  const plan = { target: annotations.videoAnnotationTarget(scene.id, item), documentSha256: hash(3), clock: 'sourcePlaybackTicks', sourceDurationTicks: 10 * T, sourceWidth: 320, sourceHeight: 180, range: item.videoEdit.range, entries: [{ annotationId: object.id, interval: object.interval, bounds: object.primitive.bounds, reference: { rasterKey: hash(4), pngSha256: hash(5), width: 80, height: 40 } }] };
  return { object, item, scene, plan };
}
function requestFor(plan, index = 0) { const entry = plan.entries[index]; return { target: plan.target, documentSha256: plan.documentSha256, annotationId: entry.annotationId, reference: entry.reference }; }
function png(width, height) {
  // Only the renderer transport header is under test; native owns full PNG decode/hash.
  const b = Buffer.alloc(72); Buffer.from([137,80,78,71,13,10,26,10]).copy(b); b.writeUInt32BE(13, 8); b.write('IHDR', 12); b.writeUInt32BE(width, 16); b.writeUInt32BE(height, 20);
  return `data:image/png;base64,${b.toString('base64')}`;
}
const reply = target => ({ ...structuredClone(target), dataUrl: png(target.reference.width, target.reference.height) });

test('source clock uses half-open intervals intersected with trim, including 100ms annotations', () => {
  const { item, scene, object } = fixture();
  assert.equal(annotations.videoAnnotationVisible(object.interval, item.videoEdit.range, 2 * T), false);
  assert.equal(annotations.videoAnnotationVisible(object.interval, item.videoEdit.range, 2.5 * T), true);
  assert.equal(annotations.videoAnnotationVisible(object.interval, item.videoEdit.range, 3 * T), false);
  assert.equal(annotations.videoAnnotationVisible({ startTicks: 27_000_000, endTicks: 28_000_000 }, item.videoEdit.range, 27_500_000), true);
  assert.equal(annotations.videoAnnotationVisible(object.interval, item.videoEdit.range, NaN), false);
  const actions = annotations.videoAnswerActions(scene, uuid(5));
  assert.equal(actions.length, 1); assert.deepEqual(actions[0].interval, { startTicks: 2.5 * T, endTicks: 3 * T });
  assert.equal(annotations.currentVideoAnswer(scene, actions[0]), item);
  item.videoEdit.revision++; assert.equal(annotations.currentVideoAnswer(scene, actions[0]), undefined);
  item.videoEdit.revision--; scene.frozen = true; assert.equal(annotations.currentVideoAnswer(scene, actions[0]), undefined);
  assert.deepEqual(annotations.videoAnswerActions(scene, 'unrelated-run'), []);
});

test('plan validates exact source/range/document order and text PNG identity', () => {
  const { plan, item, scene } = fixture();
  assert.deepEqual(preview.validateVideoAnnotationPlan(plan, scene.id, item), plan);
  for (const mutate of [p => p.target.expectedAnnotationRevision++, p => p.target.expectedRangeRevision++, p => p.entries[0].bounds.x++, p => p.entries[0].reference.width++, p => p.clock = 'framePts', p => p.extra = true]) {
    const bad = structuredClone(plan); mutate(bad); assert.throws(() => preview.validateVideoAnnotationPlan(bad, scene.id, item));
  }
  item.videoAnnotations.objects[0].primitive = { kind: 'text', topLeft: { x: 32, y: 10 }, layout: { layoutId: uuid(9), layoutSha256: hash(6), rasterSha256: hash(5), width: 80, height: 40 } };
  preview.validateVideoAnnotationPlan(plan, scene.id, item);
  const bad = structuredClone(plan); bad.entries[0].reference.pngSha256 = hash(7);
  assert.throws(() => preview.validateVideoAnnotationPlan(bad, scene.id, item));
});

test('preview rejects stale target/ref, wrong PNG dimensions, URLs and extra fields', () => {
  const expected = requestFor(fixture().plan), good = reply(expected);
  assert.deepEqual(preview.validateVideoAnnotationPreview(good, expected), good);
  for (const mutate of [p => p.target.sourceId = uuid(99), p => p.documentSha256 = hash(9), p => p.reference.rasterKey = hash(8), p => p.dataUrl = png(81, 40), p => p.dataUrl = 'https://example.invalid/a.png', p => p.extra = true]) {
    const bad = structuredClone(good); mutate(bad); assert.throws(() => preview.validateVideoAnnotationPreview(bad, expected));
  }
});

test('read broker keeps two actual requests through abandoned readers and deduplicates exact previews', async () => {
  const broker = new preview.VideoAnnotationReadBroker(2, 6), gates = [deferred(), deferred(), deferred()], starts = [];
  const jobs = gates.map((gate, i) => broker.read(() => { starts.push(i); return gate.promise; }));
  await turn(); assert.deepEqual(starts, [0, 1]); gates[0].resolve(0); await turn(); assert.deepEqual(starts, [0, 1, 2]); gates[1].resolve(1); gates[2].resolve(2); assert.deepEqual(await Promise.all(jobs), [0,1,2]);
  const target = requestFor(fixture().plan), gate = deferred(); let calls = 0;
  const a = broker.preview(target, 'asset-a', () => { calls++; return gate.promise; });
  const b = broker.preview(target, 'asset-a', () => assert.fail('duplicate'));
  assert.equal(a, b); gate.resolve(reply(target)); await a; assert.equal(calls, 1);
  await broker.preview(target, 'asset-b', async () => { calls++; return reply(target); }); assert.equal(calls, 2);
  const tiny = new preview.VideoAnnotationReadBroker(1, 1); const held = deferred(), first = tiny.read(() => held.promise);
  await assert.rejects(tiny.read(async () => 2), /繁忙/); held.resolve(1); await first;
});

test('failed preview is never cached or automatically retried, ready bytes are bounded', async () => {
  const broker = new preview.VideoAnnotationReadBroker(1, 8, 8, 1), target = requestFor(fixture().plan); let calls = 0;
  await assert.rejects(broker.preview(target, 'source', async () => { calls++; throw Error('failed'); }), /failed/);
  await turn(); assert.equal(calls, 1);
  await broker.preview(target, 'source', async () => { calls++; return reply(target); });
  await broker.preview(target, 'source', async () => { calls++; return reply(target); });
  assert.equal(calls, 3); // one-byte ready cache cannot retain either result
});

test('reader ignores late plans and images after source replacement/disposal', async () => {
  const a = fixture(), b = fixture(); b.item.asset.id = uuid(99); b.item.videoAnnotations.sourceId = uuid(99); b.plan.target.sourceId = uuid(99);
  const plans = [deferred(), deferred()], picture = deferred(), states = []; let i = 0, images = 0;
  const reader = new VideoAnnotationReader(() => plans[i++].promise, () => { images++; return picture.promise; }, state => states.push(state));
  reader.select(a.scene.id, a.item, 'source-a'); reader.at(2.6 * T);
  reader.select(b.scene.id, b.item, 'source-b'); reader.at(2.7 * T);
  plans[0].resolve(a.plan); await turn(); assert.equal(images, 0);
  plans[1].resolve(b.plan); await turn(); assert.equal(images, 1); assert.equal(states.at(-1).loading, true);
  reader.dispose(); const count = states.length; picture.resolve(reply(requestFor(b.plan))); await turn(); assert.equal(states.length, count);
});

test('reader prefetches only next second as bytes; first active set has no false-ready gap', async () => {
  const f = fixture(); f.plan.range = { startTicks: 0, endTicks: 10 * T };
  f.plan.entries.push({ ...structuredClone(f.plan.entries[0]), annotationId: uuid(10), interval: { startTicks: 3.3 * T, endTicks: 3.4 * T } }, { ...structuredClone(f.plan.entries[0]), annotationId: uuid(11), interval: { startTicks: 5 * T, endTicks: 6 * T } });
  const states = [], calls = [], first = deferred();
  const reader = new VideoAnnotationReader(async () => f.plan, target => { calls.push(target.annotationId); return target.annotationId === uuid(4) ? first.promise : Promise.resolve(reply(target)); }, state => states.push(state));
  reader.select(f.scene.id, f.item, 'source'); reader.at(2.5 * T); await turn();
  assert.deepEqual(calls, [uuid(4), uuid(10)]); assert.ok(states.every(state => state.loading));
  first.resolve(reply(requestFor(f.plan))); await turn(); assert.deepEqual([...states.at(-1).images.keys()], [uuid(4)]);
  reader.at(2.7 * T); await turn(); assert.equal(calls.length, 2);
  reader.at(3.3 * T); await turn(); assert.deepEqual([...states.at(-1).images.keys()], [uuid(10)]);
  reader.dispose();
});

test('reader byte-budget or decode failure drops partial images and needs explicit retry', async () => {
  const f = fixture(), states = []; let calls = 0;
  const reader = new VideoAnnotationReader(async () => f.plan, async target => { calls++; return reply(target); }, state => states.push(state), 1);
  reader.select(f.scene.id, f.item, 'source'); reader.at(2.6 * T); await turn();
  assert.match(states.at(-1).error, /过大/); assert.equal(states.at(-1).images, undefined);
  reader.at(2.7 * T); await turn(); assert.equal(calls, 1);
  reader.dispose();
  const ready = []; const normal = new VideoAnnotationReader(async () => f.plan, async target => reply(target), value => ready.push(value));
  normal.select(f.scene.id, f.item, 'source'); normal.at(2.6 * T); await turn(); const url = ready.at(-1).images.get(uuid(4));
  normal.imageFailed(uuid(4), 'stale-image'); assert.equal(ready.at(-1).error, undefined);
  normal.imageFailed(uuid(4), url); assert.match(ready.at(-1).error, /无法读取/); normal.dispose();
});

test('loading hold resumes only its own uninterrupted play intent, never a canceled/failed seek', async () => {
  let pauses = 0, resumes = 0, errors = 0;
  const state = { source: 'a', loading: true, failed: false, playing: true, allowed: true };
  const pause = () => pauses++, resume = async () => { resumes++; }, error = () => errors++;
  const hold = new VideoAnnotationPlaybackHold();
  hold.update(state, pause, resume, error); hold.update({ ...state, playing: false }, pause, resume, error); assert.equal(pauses, 1);
  hold.update({ ...state, loading: false, playing: false }, pause, resume, error); await turn(); assert.equal(resumes, 1);
  for (const invalidate of [h => h.invalidate(), h => h.update({ ...state, allowed: false }, pause, resume, error), h => h.update({ ...state, failed: true }, pause, resume, error), h => h.update({ ...state, source: 'b', playing: false }, pause, resume, error)]) {
    hold.update(state, pause, resume, error); invalidate(hold); hold.update({ ...state, loading: false, playing: false }, pause, resume, error);
  }
  await turn(); assert.equal(resumes, 1); assert.equal(errors, 0);
  const wait = deferred(); hold.update(state, pause, resume, error); hold.update({ ...state, loading: false }, pause, () => wait.promise, error); hold.invalidate(); wait.reject(Error('late failure')); await turn(); assert.equal(errors, 0);
});

class FakeVideo extends EventTarget {
  currentTime = 2.7; readyState = 2; seeking = false; paused = true; playCalls = 0; sourceLoads = 0; callbacks = new Map(); serial = 0;
  pause() { const changed = !this.paused; this.paused = true; if (changed) this.dispatchEvent(new Event('pause')); }
  async play() { this.playCalls++; this.paused = false; this.dispatchEvent(new Event('play')); }
  requestVideoFrameCallback(callback) { const id = ++this.serial; this.callbacks.set(id, callback); return id; }
  cancelVideoFrameCallback(id) { this.callbacks.delete(id); }
  present(time) { const [id, callback] = this.callbacks.entries().next().value; this.callbacks.delete(id); callback(0, { mediaTime: time }); }
  removeAttribute() {} load() { this.sourceLoads++; }
}
test('logical currentTime and presented PTS stay separate, held frames update via real clock without trim offset', async () => {
  const video = new FakeVideo(), seen = [], animation = new Map(); let id = 0;
  const oldRequest = globalThis.requestAnimationFrame, oldCancel = globalThis.cancelAnimationFrame;
  globalThis.requestAnimationFrame = callback => { animation.set(++id, callback); return id; };
  globalThis.cancelAnimationFrame = value => animation.delete(value);
  const player = new VideoPlayback(video, { identity: () => 'a', range: () => ({ startTicks: 2.5 * T, endTicks: 8 * T }), position() {}, playing() {}, observation: value => seen.push(value), error: error => { throw error; } });
  try {
    await player.resumeCurrent(); video.present(2.4);
    assert.deepEqual(seen.at(-1), { sourcePlaybackTicks: 2.7 * T, sourceFramePts: 2.4 * T });
    video.currentTime = 2.75; const [key, callback] = animation.entries().next().value; animation.delete(key); callback();
    assert.deepEqual(seen.at(-1), { sourcePlaybackTicks: 2.75 * T, sourceFramePts: 2.4 * T });
    player.pause(); const heldTime = video.currentTime; await player.resumeCurrent(); assert.equal(video.currentTime, heldTime); assert.equal(video.sourceLoads, 0);
    video.seeking = true; video.dispatchEvent(new Event('seeking')); assert.deepEqual(seen.at(-1), {});
    video.seeking = false; video.currentTime = 3.1; video.dispatchEvent(new Event('seeked'));
    assert.deepEqual(seen.at(-1), { sourcePlaybackTicks: 3.1 * T, sourceFramePts: undefined });
    player.pause(); const plays = video.playCalls;
    video.dispatchEvent(new Event('loadeddata')); video.dispatchEvent(new Event('loadeddata')); assert.equal(video.playCalls, plays);
  } finally { player.dispose(); globalThis.requestAnimationFrame = oldRequest; globalThis.cancelAnimationFrame = oldCancel; }
});

test('late old player unregister/seek cannot take ownership of a new source', async () => {
  const { scene } = fixture(), action = annotations.videoAnswerActions(scene, uuid(5))[0], registry = new annotations.VideoPlayerRegistry(), wait = deferred(); let alive, calls = 0;
  const unregister = registry.register(scene.id, uuid(2), { sourceIdentity: 'a', jump: async (_action, active) => { alive = active; await wait.promise; } });
  const pending = registry.jump(action, 'a', () => true); const rejected = assert.rejects(pending, /变化/);
  registry.register(scene.id, uuid(2), { sourceIdentity: 'b', jump: async () => { calls++; } }); unregister(); assert.equal(alive(), false);
  wait.resolve(); await rejected; await registry.jump(action, 'b', () => true); assert.equal(calls, 1);
  await assert.rejects(registry.jump(action, 'b', () => false), /变化/);
});

const appSource = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const appAst = parse(appSource, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
function appFunction(name) {
  let found;
  const visit = node => {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'FunctionDeclaration' && node.id?.name === name) found = node;
    for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
  };
  visit(appAst); assert.ok(found, name);
  return stripTypeScriptTypes(appSource.slice(found.start, found.end), { mode: 'transform' });
}
function actualApp() {
  const f = fixture(); Object.assign(f, { busy: false, exiting: false, recording: false, sending: false, pluginRevision: 7, operations: 0, calls: [], errors: [], flushWait: undefined, mutation: undefined });
  f.scene.draft = 'preserve draft'; f.scene.refs = [{ kind: 'item', id: f.item.id }];
  const scope = { ...annotations, TrimCanceled, disposed: false, commands: Promise.resolve(), exitFailure: undefined, videoCommits: new Set(), videoFlushDepth: 0, assertVideoTextDraftsSaved,
    scene: () => f.scene, busy: () => f.busy, sending: () => f.sending, pluginSnapshot: () => ({ revision: f.pluginRevision }), recording: () => f.recording, scroll: () => false, exitPreparing: () => f.exiting, queueMicrotask,
    setBusy: value => { f.busy = value; }, showError: error => f.errors.push(error),
    flush: async () => { f.calls.push('flush'); if (f.flushWait) await f.flushWait.promise; },
    spaceOperations: { begin() { f.operations++; return () => { f.operations--; }; } },
    focusReference: ref => f.calls.push(['focus', ref.id]),
    videoNavigation: { async jump(action, source, active) { assert.ok(active()); assert.equal(f.busy, false); assert.equal(f.operations, 0); f.calls.push(['jump', action.annotationId, source]); f.playerActive = active; if (f.playerWait) await f.playerWait.promise; } },
    videoAnnotationBridge: { async applyVideoAnnotationDocument(target, action) {
      f.calls.push(['mutation', structuredClone(target), structuredClone(action)]);
      if (f.mutation) return f.mutation.promise;
      const next = structuredClone(f.scene); next.items[0].videoAnnotations.revision++;
      next.items[0].videoAnnotations.objects = [];
      return { scenes: [next] };
    } },
    accept: snapshot => { f.scene = snapshot.scenes[0]; f.item = f.scene.items[0]; },
  };
  vm.createContext(scope); vm.runInContext(`${appFunction('applyVideoAnnotation')}\n${appFunction('jumpToVideoAnswer')}\n${appFunction('flushVideo')}`, scope);
  return { f, scope };
}

test('actual App releases preparation busy before local playback and retains committed provenance', async () => {
  const { f, scope } = actualApp(), action = annotations.videoAnswerActions(f.scene, uuid(5))[0];
  const before = JSON.stringify([f.scene.draft, f.scene.refs]); f.flushWait = deferred();
  const jump = scope.jumpToVideoAnswer(action); await turn(); assert.deepEqual(f.calls, ['flush']); assert.equal(f.operations, 1);
  f.flushWait.resolve(); await jump; assert.deepEqual(f.calls.map(value => Array.isArray(value) ? value[0] : value), ['flush', 'focus', 'jump']);
  assert.equal(JSON.stringify([f.scene.draft, f.scene.refs]), before); assert.equal(f.busy, false); assert.equal(f.operations, 0);
  for (const change of [f => f.scene.frozen = true, f => f.item.asset.width = 200, f => f.item.videoAnnotations.revision++, f => f.item.videoAnnotations.objects[0].primitive.style.color = '#00ff00', f => f.item.videoEdit.revision++, f => f.exiting = true, f => f.pluginRevision++, f => f.sending = true]) {
    const { f, scope } = actualApp(); f.flushWait = deferred(); const jump = scope.jumpToVideoAnswer(annotations.videoAnswerActions(f.scene, uuid(5))[0]);
    change(f); f.flushWait.resolve(); await jump; assert.deepEqual(f.calls, ['flush']); assert.equal(f.operations, 0);
  }
});

test('actual App phase handoff waits busy=false effects and plugin epoch revokes local pending playback', async () => {
  const { f, scope } = actualApp(); f.playerWait = deferred(); let released = false;
  scope.setBusy = value => { f.busy = value; if (!value) queueMicrotask(() => { released = true; }); };
  const originalJump = scope.videoNavigation.jump;
  scope.videoNavigation.jump = async (...args) => { assert.equal(released, true); await originalJump(...args); };
  const pending = scope.jumpToVideoAnswer(annotations.videoAnswerActions(f.scene, uuid(5))[0]); await turn();
  assert.equal(f.busy, false); assert.equal(f.operations, 0); assert.ok(f.playerActive());
  f.pluginRevision++; assert.equal(f.playerActive(), false); f.playerWait.resolve(); await pending;
  const second = actualApp(); second.scope.setBusy = value => {
    second.f.busy = value;
    if (!value) queueMicrotask(() => { second.f.pluginRevision++; });
  };
  await second.scope.jumpToVideoAnswer(annotations.videoAnswerActions(second.f.scene, uuid(5))[0]);
  assert.deepEqual(second.f.calls.map(value => Array.isArray(value) ? value[0] : value), ['flush', 'focus']);
});

test('actual plugin snapshot update pauses all players on a new epoch without deleting committed annotations', () => {
  let arrow;
  const visit = node => {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'VariableDeclarator' && node.id?.name === 'acceptPlugins') arrow = node.init;
    for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
  };
  visit(appAst); assert.ok(arrow);
  let snapshot = { revision: 7, plugins: [] }; const calls = [], current = fixture(), doc = JSON.stringify(current.item.videoAnnotations);
  const context = { disposed: false, pluginSnapshot: () => snapshot, setPluginSnapshot: next => { calls.push('accept'); snapshot = next; }, videoPlayers: { pauseAll: () => calls.push('pause') }, voice: { reconcile() {} }, recordingAudio: { reconcile() {} }, recordingAudioGrants: value => value };
  vm.createContext(context); vm.runInContext(stripTypeScriptTypes(`const acceptPlugins = ${appSource.slice(arrow.start, arrow.end)};`, { mode: 'transform' }), context);
  const accept = vm.runInContext('acceptPlugins', context);
  accept({ revision: 6, plugins: [] }); assert.deepEqual(calls, []);
  accept({ revision: 7, plugins: [] }); assert.deepEqual(calls, ['accept']);
  calls.length = 0; accept({ revision: 8, plugins: [] }); assert.deepEqual(calls, ['pause', 'accept']);
  assert.equal(JSON.stringify(current.item.videoAnnotations), doc);
});

test('actual App failed navigation flush never focuses/seeks or consumes the draft', async () => {
  const { f, scope } = actualApp(); f.flushWait = deferred(); const old = JSON.stringify([f.scene.draft, f.scene.refs]);
  const jump = scope.jumpToVideoAnswer(annotations.videoAnswerActions(f.scene, uuid(5))[0]);
  f.flushWait.reject(Error('synthetic flush failed')); await jump;
  assert.deepEqual(f.calls, ['flush']); assert.equal(f.errors.length, 1);
  assert.equal(f.operations, 0); assert.equal(f.busy, false); assert.equal(JSON.stringify([f.scene.draft, f.scene.refs]), old);
});

test('actual App document edit needs no plugin, waits real mutation before exit flush and rejects stale history', async () => {
  const { f, scope } = actualApp(), target = annotations.videoAnnotationTarget(f.scene.id, f.item);
  f.mutation = deferred(); const edit = scope.applyVideoAnnotation(target, { type: 'remove', annotationId: uuid(4) }); await turn();
  assert.equal(f.operations, 1); assert.equal(scope.videoCommits.size, 1);
  scope.videoFlush = { async flush() {} }; let drained = false;
  const flush = scope.flushVideo(() => true).then(() => { drained = true; }); await turn(); assert.equal(drained, false);
  const result = structuredClone(f.scene); result.items[0].videoAnnotations.revision++; result.items[0].videoAnnotations.objects = [];
  f.exiting = true; f.mutation.resolve({ scenes: [result] }); await edit; await flush;
  assert.equal(f.item.videoAnnotations.revision, target.expectedAnnotationRevision + 1); assert.equal(f.operations, 0); assert.equal(scope.videoCommits.size, 0);
  assert.equal(scope.videoFlushDepth, 0);
  assert.equal(f.scene.draft, 'preserve draft');
  const other = actualApp(); await assert.rejects(other.scope.applyVideoAnnotation(annotations.videoAnnotationTarget(other.f.scene.id, other.f.item), { type: 'undo', operationId: uuid(80) }), /历史/);
  assert.deepEqual(other.f.calls, []);
});

test('actual App queued document edit is rejected after source/range/scene/run or capture changes', async () => {
  for (const change of [f => f.item.videoAnnotations.revision++, f => f.item.videoEdit.revision++, f => f.scene.id = uuid(99), f => f.scene.run = { status: 'running' }, f => f.recording = true]) {
    const { f, scope } = actualApp(), wait = deferred(); scope.commands = wait.promise;
    const edit = scope.applyVideoAnnotation(annotations.videoAnnotationTarget(f.scene.id, f.item), { type: 'remove', annotationId: uuid(4) });
    const rejected = assert.rejects(edit, /已更新/); change(f); wait.resolve(); await rejected;
    assert.deepEqual(f.calls, []); assert.equal(f.operations, 0);
  }
});
