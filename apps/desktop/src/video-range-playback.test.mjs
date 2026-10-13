// SPDX-License-Identifier: MPL-2.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';

const moduleUrls = new Map();
async function load(name) {
  if (moduleUrls.has(name)) return moduleUrls.get(name);
  let code = stripTypeScriptTypes(await readFile(new URL(`./${name}.ts`, import.meta.url), 'utf8'), { mode: 'transform' });
  for (const dependency of [...code.matchAll(/from ['"]\.\/([^'"]+)['"]/g)].map(value => value[1])) code = code.replaceAll(`'./${dependency}'`, JSON.stringify(await load(dependency))).replaceAll(`"./${dependency}"`, JSON.stringify(await load(dependency)));
  const url = `data:text/javascript;base64,${Buffer.from(`${code}\n//# sourceURL=${name}.ts`).toString('base64')}`;
  moduleUrls.set(name, url); return url;
}
const { VideoPlayback } = await import(await load('video-media'));
const { VideoAnnotationPlaybackHold } = await import(await load('video-annotation-playback'));
const annotations = await import(await load('video-annotations'));
const { TrimCanceled } = await import(await load('video-trim'));
const T = 10_000_000;
const turn = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
class Media extends EventTarget {
  time = 2.5; readyState = 2; seeking = false; paused = true; assignments = []; playCalls = 0; playWaits = []; sourceLoads = 0; frames = new Map(); serial = 0; queued = false; mediaEvents = [];
  get currentTime() { return this.time; }
  set currentTime(value) { this.time = value; this.seeking = true; this.assignments.push(value); this.dispatchEvent(new Event('seeking')); }
  finishSeek(actual = this.time) { this.time = actual; this.seeking = false; this.dispatchEvent(new Event('seeked')); }
  emit(type) { if (this.queued) this.mediaEvents.push(type); else this.dispatchEvent(new Event(type)); }
  dispatchQueued(type) { const index = this.mediaEvents.indexOf(type); assert.ok(index >= 0, type); this.mediaEvents.splice(index, 1); this.dispatchEvent(new Event(type)); }
  pause() { const changed = !this.paused; this.paused = true; if (changed) this.emit('pause'); }
  play() {
    this.playCalls++;
    const wait = this.playWaits.shift();
    const start = () => { this.paused = false; this.emit('play'); };
    if (wait) return wait.promise.then(start); start(); return Promise.resolve();
  }
  requestVideoFrameCallback(callback) { const id = ++this.serial; this.frames.set(id, callback); return id; }
  cancelVideoFrameCallback(id) { this.frames.delete(id); }
  present(time) { const [id, callback] = this.frames.entries().next().value; this.frames.delete(id); callback(0, { mediaTime: time }); }
  removeAttribute() {} load() {
    this.sourceLoads++;
    if (!this.ended) return;
    this.ended = false; this.readyState = 0; this.time = 0; this.seeking = false; this.paused = true;
    this.dispatchEvent(new Event('emptied'));
    queueMicrotask(() => { this.readyState = 2; this.dispatchEvent(new Event('loadedmetadata')); });
  }
}
function fixture() {
  const node = new Media(), positions = [], observations = [], errors = []; let source = 'original-source', range = { startTicks: 2.5 * T, endTicks: 8 * T }, active = true, ready = true;
  const player = new VideoPlayback(node, { identity: () => source, range: () => range, position: value => positions.push(value), playing() {}, observation: value => observations.push(value), error: value => errors.push(value) });
  return { node, player, positions, observations, errors, isActive: () => active, isReady: () => ready, get range() { return range; }, set range(value) { range = value; }, set source(value) { source = value; }, set active(value) { active = value; }, set ready(value) { ready = value; } };
}
async function start(f, interval = { startTicks: 25_300_000, endTicks: 26_300_000 }) {
  const pending = f.player.playInterval(interval, f.isActive, f.isReady); await turn();
  f.node.finishSeek(); await pending;
}
function animations() {
  const saved = [globalThis.requestAnimationFrame, globalThis.cancelAnimationFrame], frames = new Map(); let serial = 0;
  globalThis.requestAnimationFrame = callback => { frames.set(++serial, callback); return serial; };
  globalThis.cancelAnimationFrame = id => frames.delete(id);
  return { frames, run() { const [id, callback] = frames.entries().next().value; frames.delete(id); callback(0); }, restore() { [globalThis.requestAnimationFrame, globalThis.cancelAnimationFrame] = saved; } };
}

test('an explicit pause synchronizes observed native pause before queued events, including an already paused trim boundary', async () => {
  for (const phase of ['queued-event', 'already-paused', 'trim-boundary']) {
    const node = new Media(), states = [];
    const player = new VideoPlayback(node, { identity: () => 'source', range: () => ({ startTicks: 0, endTicks: 8 * T }), position() {}, playing: value => states.push(value), error: error => { throw error; } });
    try {
      node.paused = false; node.dispatchEvent(new Event('play')); assert.equal(states.at(-1), true);
      node.queued = true;
      if (phase === 'already-paused') node.paused = true;
      if (phase === 'trim-boundary') { node.time = 8; node.dispatchEvent(new Event('timeupdate')); }
      else player.pause();
      assert.equal(node.paused, true, phase); assert.equal(states.at(-1), false, phase);
      assert.equal(node.sourceLoads, 0, phase);
      if (node.mediaEvents.includes('pause')) node.dispatchQueued('pause');
      assert.equal(states.at(-1), false, phase);
    } finally { player.dispose(); await turn(); }
  }
});

test('pause never publishes a false state for a retired source, a refused native pause or a reentrant newer play', async () => {
  for (const phase of ['retired-source', 'refused-pause', 'new-play']) {
    const node = new Media(), states = []; let source = 'source', next;
    const player = new VideoPlayback(node, { identity: () => source, range: () => ({ startTicks: 0, endTicks: 8 * T }), position() {}, playing: value => states.push(value), error: error => { throw error; } });
    try {
      node.paused = false; node.dispatchEvent(new Event('play')); node.queued = true;
      if (phase === 'retired-source') source = 'new-source';
      if (phase === 'refused-pause') node.pause = () => {};
      if (phase === 'new-play') node.pause = () => { node.paused = true; next = player.resumeCurrent(); };
      const count = states.length; player.pause();
      if (next) { await next; node.dispatchQueued('play'); }
      assert.equal(states.slice(count).includes(false), false, phase);
      assert.equal(states.at(-1), true, phase);
    } finally { node.pause = Media.prototype.pause; player.dispose(); }
  }
});

test('disposed media never notifies the retired view owner and drains listeners, frames and a pending seek', async () => {
  class TrackedMedia extends Media {
    listeners = new Map();
    addEventListener(name, callback, options) {
      const listeners = this.listeners.get(name) ?? new Set(); listeners.add(callback); this.listeners.set(name, listeners);
      super.addEventListener(name, callback, options);
    }
    removeEventListener(name, callback, options) {
      this.listeners.get(name)?.delete(callback); super.removeEventListener(name, callback, options);
    }
    get listenerCount() { return [...this.listeners.values()].reduce((total, listeners) => total + listeners.size, 0); }
  }
  const node = new TrackedMedia(), raf = animations(); let ownerAlive = true, notifications = 0;
  const notify = () => { assert.equal(ownerAlive, true, 'a terminal media callback accessed its retired view owner'); notifications++; };
  const player = new VideoPlayback(node, { identity: () => 'source', range: () => ({ startTicks: 0, endTicks: 8 * T }), position: notify, playing: notify, observation: notify, error: notify });
  try {
    node.paused = false; node.dispatchEvent(new Event('play'));
    assert.ok(notifications > 0); assert.ok(node.frames.size > 0); assert.ok(raf.frames.size > 0);
    player.seek(3 * T); await turn(); assert.equal(node.seeking, true);
    // Native HTML media events are queued tasks. Disposal must remove their
    // listeners and abort the in-flight seek before any late event can run.
    node.queued = true; ownerAlive = false; const before = notifications;
    player.dispose(); await turn();
    assert.equal(notifications, before); assert.equal(node.listenerCount, 0);
    assert.equal(node.frames.size, 0); assert.equal(raf.frames.size, 0); assert.equal(node.sourceLoads, 1);
    node.finishSeek(); node.dispatchEvent(new Event('loadedmetadata')); node.dispatchEvent(new Event('pause')); node.dispatchEvent(new Event('error'));
    await turn(); assert.equal(notifications, before); assert.equal(node.listenerCount, 0);
    await assert.rejects(player.seeks.flush(() => true), TrimCanceled);
    player.dispose(); assert.equal(node.sourceLoads, 1);
  } finally { ownerAlive = true; player.dispose(); raf.restore(); }
});

test('non-frame-aligned 100ms interval waits real seek and PNG, held-frame RAF stops at end without trim offset', async () => {
  const f = fixture(), raf = animations(), interval = { startTicks: 25_300_000, endTicks: 26_300_000 }, before = structuredClone(f.range);
  f.ready = false;
  try {
    const pending = f.player.playInterval(interval, f.isActive, f.isReady); await turn();
    assert.deepEqual(f.node.assignments, [2.53]); assert.equal(f.node.playCalls, 0);
    f.node.finishSeek(); await turn(); assert.equal(f.node.playCalls, 0);
    f.ready = true; await pending; assert.equal(f.node.paused, false);
    f.node.present(2.4); f.node.time = 2.6; raf.run();
    assert.deepEqual(f.observations.at(-1), { sourcePlaybackTicks: 2.6 * T, sourceFramePts: 2.4 * T });
    f.node.time = 2.7; raf.run(); await turn();
    assert.equal(f.node.paused, true); assert.equal(raf.frames.size, 0); assert.equal(f.node.assignments.at(-1), 2.63);
    f.node.finishSeek(); await f.player.seeks.flush(); assert.equal(f.node.currentTime, 2.63);
    assert.deepEqual(f.range, before); assert.equal(f.node.sourceLoads, 0); assert.deepEqual(f.errors, []);
  } finally { f.player.dispose(); raf.restore(); }
});

test('pending seek or raster cannot revive after user pause, source/trim/scope change, scrub or disposal', async () => {
  const changes = [f => f.player.pause(), f => { f.active = false; }, f => { f.source = 'other-source'; f.player.invalidate(); }, f => { f.range = { startTicks: 2.6 * T, endTicks: 7 * T }; }, f => f.player.seek(4 * T), f => f.player.dispose()];
  for (const stage of ['seek', 'raster']) for (const change of changes) {
    const f = fixture(); f.ready = false;
    try {
      const pending = f.player.playInterval({ startTicks: 25_300_000, endTicks: 26_300_000 }, f.isActive, f.isReady), rejected = assert.rejects(pending, TrimCanceled);
      await turn();
      if (stage === 'raster') { f.node.finishSeek(); await turn(); }
      change(f); f.ready = true; f.node.finishSeek(); await rejected;
      await turn(); if (f.node.seeking) f.node.finishSeek();
      assert.equal(f.node.playCalls, 0, stage); assert.equal(f.node.paused, true, stage);
    } finally { f.player.dispose(); }
  }
});

test('out-of-window seek and stale or out-of-trim intervals never play', async () => {
  const f = fixture();
  try {
    for (const interval of [{ startTicks: 2.4 * T, endTicks: 2.7 * T }, { startTicks: 2.6 * T, endTicks: 8.1 * T }, { startTicks: 3 * T, endTicks: 3 * T }, { startTicks: 3.01, endTicks: 4 * T }]) await assert.rejects(f.player.playInterval(interval, f.isActive, f.isReady), TrimCanceled);
    const pending = f.player.playInterval({ startTicks: 25_300_000, endTicks: 25_400_000 }, f.isActive, f.isReady), rejected = assert.rejects(pending, /定位/);
    await turn(); f.node.finishSeek(2.545); await rejected; assert.equal(f.node.playCalls, 0); assert.equal(f.node.paused, true);
  } finally { f.player.dispose(); }
});

test('raster hold preserves the interval and position, canceled hold or end cannot restart ordinary playback', async () => {
  const f = fixture(), hold = new VideoAnnotationPlaybackHold(), errors = [];
  const update = loading => hold.update({ source: 'document-a', intent: f.player.playIntent(), loading, failed: false, playing: !f.node.paused, allowed: true }, () => f.player.holdForRaster(), () => f.player.resumeCurrent(), error => errors.push(error));
  try {
    await start(f); const seeks = f.node.assignments.length;
    f.node.time = 2.57; f.ready = false; update(true); assert.equal(f.node.paused, true);
    f.ready = true; update(false); await turn(); assert.equal(f.node.paused, false); assert.equal(f.node.currentTime, 2.57); assert.equal(f.node.assignments.length, seeks);
    f.ready = false; update(true); f.player.pause(); f.ready = true; update(false); await turn(); assert.equal(f.node.paused, true);
    await start(f); f.ready = false; update(true); const plays = f.node.playCalls;
    f.node.time = 2.64; f.node.dispatchEvent(new Event('timeupdate')); await turn(); f.node.finishSeek();
    f.ready = true; update(false); await turn(); assert.equal(f.node.playCalls, plays); assert.equal(f.node.paused, true); assert.deepEqual(errors, []);
  } finally { f.player.dispose(); }
});

test('late play acknowledgement pauses a revoked owner and cannot pause a newer accepted intent', async () => {
  for (const newer of [false, true]) {
    const f = fixture(), first = deferred(); f.node.playWaits.push(first);
    try {
      const pending = f.player.resumeCurrent(), rejected = assert.rejects(pending, TrimCanceled); await turn();
      f.player.pause();
      if (newer) { const next = f.player.play(); await turn(); f.node.finishSeek(); await next; assert.equal(f.node.paused, false); }
      first.resolve(); await rejected;
      assert.equal(f.node.paused, !newer); assert.equal(f.node.sourceLoads, 0);
    } finally { f.player.dispose(); }
  }
});

test('play restarts from the retained beginning after source end or an explicit timeline endpoint', async () => {
  for (const action of ['button-end', 'timeline-end', 'rounded-native-end']) {
    const f = fixture();
    try {
      f.node.time = action === 'rounded-native-end' ? 7.999 : 8;
      f.node.ended = action === 'rounded-native-end';
      const pending = f.player.play(action === 'timeline-end' ? f.range.endTicks : undefined);
      await turn();
      assert.equal(f.node.assignments.at(-1), 2.5, action);
      f.node.finishSeek(); await pending;
      assert.equal(f.node.paused, false, action);
      assert.equal(f.node.playCalls, 1);
      assert.equal(f.node.sourceLoads, action === 'rounded-native-end' ? 1 : 0);
      assert.deepEqual(f.errors, []);
    } finally { f.player.dispose(); }
  }
});

test('an explicit valid resume position wins over ended while canceled replay cannot start playback', async () => {
  const f = fixture();
  try {
    f.node.ended = true;
    const resumed = f.player.play(4 * T); await turn();
    assert.equal(f.node.assignments.at(-1), 4);
    f.node.finishSeek(); await resumed;
    f.player.pause(); f.node.time = 8;
    const replay = f.player.play(f.range.endTicks), rejected = assert.rejects(replay, TrimCanceled);
    await turn(); f.player.invalidate(); f.node.finishSeek(); await rejected;
    assert.equal(f.node.playCalls, 1);
    assert.equal(f.node.paused, true);
    assert.equal(f.node.sourceLoads, 1, 'a real ended source resets once, including explicit retained resume');
  } finally { f.player.dispose(); }
});

test('completed trim replays after its boundary seek lands just before the endpoint', async () => {
  const f = fixture(); f.range = { startTicks: 0, endTicks: 93_213_559 };
  try {
    f.node.time = 9.4; f.node.paused = false;
    f.node.dispatchEvent(new Event('timeupdate')); await turn();
    assert.equal(f.node.paused, true);
    f.node.finishSeek(9.321); await f.player.seeks.flush();
    assert.equal(f.node.currentTime, 9.321);
    const replay = f.player.play(); await turn();
    const assigned = f.node.assignments.at(-1);
    f.node.finishSeek(); await replay;
    assert.equal(assigned, 0);
    assert.equal(f.node.paused, false); assert.deepEqual(f.errors, []);
  } finally { f.player.dispose(); }
});

test('user seek, explicit resume and range replacement clear or supersede trim completion', async () => {
  for (const action of ['seek', 'explicit-resume', 'range', 'source']) {
    const f = fixture();
    try {
      f.node.time = 8.1; f.node.paused = false;
      f.node.dispatchEvent(new Event('timeupdate')); await turn();
      f.node.finishSeek(7.999); await f.player.seeks.flush();
      if (action === 'seek') { f.player.seek(7.999 * T); await turn(); f.node.finishSeek(); await f.player.seeks.flush(); }
      if (action === 'range') f.range = { startTicks: 1 * T, endTicks: 9 * T };
      if (action === 'source') { f.source = 'replacement-source'; f.player.invalidate(); f.node.dispatchEvent(new Event('loadedmetadata')); }
      const replay = f.player.play(action === 'explicit-resume' ? 7.999 * T : undefined); await turn();
      assert.equal(f.node.assignments.at(-1), 7.999, action);
      f.node.finishSeek(); await replay; assert.equal(f.node.paused, false, action);
    } finally { f.player.dispose(); }
  }
});

test('pausing preserves a completed trim but a canceled replay never revives playback', async () => {
  const f = fixture();
  try {
    f.node.time = 8.1; f.node.paused = false;
    f.node.dispatchEvent(new Event('timeupdate')); await turn();
    f.node.finishSeek(7.999); await f.player.seeks.flush();
    f.player.pause(); f.node.dispatchEvent(new Event('pause'));
    const pending = f.player.play(), rejected = assert.rejects(pending, TrimCanceled);
    await turn(); assert.equal(f.node.assignments.at(-1), 2.5);
    f.player.pause(); f.node.finishSeek(); await rejected;
    assert.equal(f.node.paused, true); assert.equal(f.node.playCalls, 0);
  } finally { f.player.dispose(); }
});

test('interval ending at trim/source endpoint never requests a seek outside retained source', async () => {
  const f = fixture(); f.range = { startTicks: 2.5 * T, endTicks: 3 * T };
  try {
    await start(f, { startTicks: 2.9 * T, endTicks: 3 * T });
    f.node.time = 3.05; f.node.dispatchEvent(new Event('timeupdate')); await turn();
    assert.equal(f.node.paused, true); assert.equal(f.node.assignments.at(-1), 3); f.node.finishSeek();
    assert.ok(f.node.assignments.every(time => time >= 2.5 && time <= 3));
  } finally { f.player.dispose(); }
});

test('endpoint correction that actually lands beyond the exclusive end is a failed seek, never reported as success', async () => {
  const f = fixture();
  try {
    await start(f); f.node.time = 2.64; f.node.dispatchEvent(new Event('timeupdate')); await turn();
    f.node.finishSeek(2.635); await assert.rejects(f.player.seeks.flush(), /定位/);
    assert.equal(f.node.paused, true); assert.equal(f.errors.length, 1); assert.equal(f.player.hasIntervalIntent(), false);
  } finally { f.player.dispose(); }
});

test('latest interval owns the pending seek and blur-style pause cancels a queued endpoint correction', async () => {
  const f = fixture();
  try {
    const old = f.player.playInterval({ startTicks: 25_300_000, endTicks: 26_300_000 }, f.isActive, f.isReady), rejected = assert.rejects(old, TrimCanceled); await turn();
    const next = f.player.playInterval({ startTicks: 4 * T, endTicks: 5 * T }, f.isActive, f.isReady); await turn();
    assert.equal(f.node.currentTime, 4); f.node.finishSeek(); await next; await rejected; assert.equal(f.node.playCalls, 1);
    const assignments = f.node.assignments.length;
    f.node.time = 5.1; f.node.dispatchEvent(new Event('timeupdate')); f.player.pause(); await turn();
    assert.equal(f.node.assignments.length, assignments); assert.equal(f.node.paused, true);
    assert.equal(f.player.hasIntervalIntent(), false);
  } finally { f.player.dispose(); }
});

test('queued old pause event cannot cancel a new seek/raster wait or erase a newer playing RAF', async () => {
  for (const phase of ['seek', 'raster', 'playing']) {
    const f = fixture(), raf = animations(); f.node.queued = true;
    try {
      await f.player.resumeCurrent(); f.node.dispatchQueued('play'); assert.equal(raf.frames.size, 1);
      f.player.pause(); f.ready = phase !== 'raster';
      const pending = f.player.playInterval({ startTicks: 25_300_000, endTicks: 26_300_000 }, f.isActive, f.isReady);
      await turn();
      if (phase !== 'seek') { f.node.finishSeek(); await turn(); }
      if (phase === 'playing') { await pending; f.node.dispatchQueued('play'); assert.equal(raf.frames.size, 1); }
      f.node.dispatchQueued('pause'); assert.equal(f.player.hasIntervalIntent(), true, phase);
      if (phase === 'playing') { assert.equal(f.node.paused, false); assert.equal(raf.frames.size, 1); }
      else { f.ready = true; if (phase === 'seek') f.node.finishSeek(); await pending; assert.equal(f.node.paused, false); }
    } finally { f.player.dispose(); raf.restore(); }
  }
});

test('queued stale seeking/play events cannot revoke a completed seek or report a paused raster hold as playing', async () => {
  const node = new Media(), playing = [], range = { startTicks: 25_000_000, endTicks: 30_000_000 };
  const player = new VideoPlayback(node, { identity: () => 'source', range: () => range, position() {}, playing: value => playing.push(value), error: error => { throw error; } });
  try {
    const pending = player.playInterval({ startTicks: 25_300_000, endTicks: 26_300_000 }, () => true, () => true); await turn(); node.finishSeek(); await pending;
    node.dispatchEvent(new Event('seeking')); assert.equal(player.hasIntervalIntent(), true); assert.equal(node.paused, false);
    player.holdForRaster(); assert.equal(node.paused, true); const count = playing.length;
    node.dispatchEvent(new Event('play')); assert.equal(playing.length, count); assert.equal(playing.at(-1), false); assert.equal(player.hasIntervalIntent(), true);
  } finally { player.dispose(); }
});

// Execute the actual component's registered port function, not a replica.
const componentSource = await readFile(new URL('./components/VideoArtifact.tsx', import.meta.url), 'utf8');
const ast = parse(componentSource, { sourceType: 'module', plugins: ['typescript', 'jsx'] }); let port, toggle, mediaContextMenu;
function visit(node) {
  if (!node || typeof node !== 'object') return;
  if (node.type === 'ObjectProperty' && node.key?.name === 'jump' && node.value?.type === 'ArrowFunctionExpression') port = node.value;
  if (node.type === 'VariableDeclarator' && node.id?.name === 'toggle') toggle = node.init;
  if (node.type === 'JSXOpeningElement' && node.name?.name === 'video') mediaContextMenu = node.attributes.find(attribute => attribute.name?.name === 'onContextMenu')?.value?.expression;
  for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
}
visit(ast); assert.ok(port);

test('actual component foreign-view callbacks ignore retired owners before reading range or media', () => {
  const constructors = [];
  function collect(node) {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'NewExpression') constructors.push(node);
    for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(collect); else if (value && typeof value === 'object') collect(value);
  }
  collect(ast);
  const property = (constructor, index, name) => constructors.find(node => node.callee?.name === constructor)?.arguments[index]?.properties.find(node => node.key?.name === name)?.value;
  const callbacks = {
    position: property('VideoPlayback', 1, 'position'), playing: property('VideoPlayback', 1, 'playing'),
    observation: property('VideoPlayback', 1, 'observation'), manual: property('VideoManualEditor', 0, 'publish'),
    reader: constructors.find(node => node.callee?.name === 'VideoAnnotationReader')?.arguments[2],
  };
  let reads = 0; const calls = [];
  const live = () => { assert.equal(context.disposed, false, 'retired callback read view authority'); reads++; };
  const context = { disposed: false, videoAnnotationPreviewTicks: annotations.videoAnnotationPreviewTicks,
    range: () => { live(); return { startTicks: 0, endTicks: T }; },
    video: { get paused() { live(); return true; }, get seeking() { live(); return false; } },
    setPosition: value => calls.push(['position', value]), setPlaying: value => calls.push(['playing', value]),
    setAnnotationPosition: value => calls.push(['observation', value]), annotationReader: { at: value => calls.push(['at', value]) },
    setManualView: value => calls.push(['manual', value]), setAnnotationState: value => calls.push(['reader', value]),
  };
  vm.createContext(context);
  for (const [name, callback] of Object.entries(callbacks)) {
    assert.ok(callback, name);
    vm.runInContext(stripTypeScriptTypes(`globalThis.${name} = ${componentSource.slice(callback.start, callback.end)};`, { mode: 'transform' }), context);
  }
  const invoke = () => { context.position(0); context.playing(false); context.observation({ sourcePlaybackTicks: 0 }); context.manual({ pending: false }); context.reader({ loading: false }); };
  invoke(); assert.equal(calls.length, 6); assert.equal(reads, 3);
  context.disposed = true; invoke(); assert.equal(calls.length, 6); assert.equal(reads, 3);
});
test('the real video context-menu handler suppresses browser commands without starting playback', () => {
  assert.ok(mediaContextMenu);
  const event = new Event('contextmenu', { cancelable: true }); let propagation = 0;
  event.stopPropagation = () => { propagation++; };
  const context = { event }; vm.createContext(context);
  vm.runInContext(stripTypeScriptTypes(`const menu = ${componentSource.slice(mediaContextMenu.start, mediaContextMenu.end)}; menu(event);`, { mode: 'transform' }), context);
  assert.equal(event.defaultPrevented, true);
  assert.equal(propagation, 1);
});
const portCode = stripTypeScriptTypes(`const jump = ${componentSource.slice(port.start, port.end)};`, { mode: 'transform' });
function componentPort() {
  const object = { id: 'annotation', interval: { startTicks: 2 * T, endTicks: 3 * T }, origin: { runId: 'run', toolEventId: 'tool' } };
  const item = { id: 'item', asset: { id: 'source', kind: 'video' }, videoEdit: { revision: 1, range: { startTicks: 2.5 * T, endTicks: 8 * T } }, videoAnnotations: { revision: 1, objects: [object] } };
  const props = { sceneId: 'scene', item, active: true, busy: false, exporting: undefined, annotationBusy: false };
  const action = { target: annotations.videoAnnotationTarget(props.sceneId, item), annotationId: object.id, runId: 'run', toolEventId: 'tool', interval: { startTicks: 2.5 * T, endTicks: 3 * T } };
  const wait = deferred(), calls = [], plan = { target: action.target, documentSha256: 'original-native-document' };
  const sourceKey = () => annotations.videoSourceIdentity(props.sceneId, item), annotationKey = () => annotations.videoAnnotationIdentity(props.sceneId, item);
  const context = { ...annotations, TrimCanceled, disposed: false, props, sourceKey, annotationKey, key: sourceKey(), range: () => item.videoEdit.range, annotationState: () => ({ plan }), annotationUnavailable: () => false,
    pausePlayback: () => calls.push('pause'), setSelectedAnnotation: id => calls.push(['selected', id]), setDrawingOpen: value => { assert.equal(value, false); context.closedDrawing = true; },
    playback: { async playInterval(interval, active, ready) { calls.push(structuredClone(interval)); context.valid = active; assert.equal(ready(), true); await wait.promise; if (!active()) throw new TrimCanceled(); } } };
  vm.createContext(context); vm.runInContext(portCode, context);
  return { props, item, action, context, wait, plan, calls, jump: (...args) => vm.runInContext('jump', context)(...args) };
}
test('actual component port rejects noncommitted intervals and tracks full document, native preview and scope through playback', async () => {
  for (const change of [f => { f.item.videoAnnotations.objects[0].origin.toolEventId = 'other'; }, f => { f.action.interval.endTicks = 4 * T; }, f => { f.props.active = false; }, f => { f.props.exporting = {}; }]) {
    const f = componentPort(); change(f); await assert.rejects(f.jump(f.action, () => true), TrimCanceled); assert.deepEqual(f.calls, []);
  }
  for (const change of [f => { f.props.busy = true; }, f => { f.props.annotationBusy = true; }, f => { f.props.exporting = {}; }, f => { f.item.asset.id = 'new'; }, f => { f.item.videoAnnotations.objects[0].color = 'changed-without-revision'; }, f => { f.item.videoEdit.revision++; }, f => { f.plan.documentSha256 = 'different-native-document'; }]) {
    const f = componentPort(), pending = f.jump(f.action, () => true), rejected = assert.rejects(pending, TrimCanceled);
    assert.equal(f.context.valid(), true); change(f); assert.equal(f.context.valid(), false); f.wait.resolve(); await rejected;
  }
});

test('actual video click/space toggle cancels an interval even while paused waiting for raster', () => {
  assert.ok(toggle); let pauses = 0, plays = 0;
  const context = { playback: { hasIntervalIntent: () => true, play: () => { plays++; return Promise.resolve(); } }, pausePlayback: () => { pauses++; }, previewHold: { invalidate() {} }, props: { busy: false }, annotationUnavailable: () => true, range: () => ({ startTicks: 0, endTicks: T }), playing: () => false, report() {} };
  vm.createContext(context); vm.runInContext(stripTypeScriptTypes(`const toggle = ${componentSource.slice(toggle.start, toggle.end)}; toggle();`, { mode: 'transform' }), context);
  assert.equal(pauses, 1); assert.equal(plays, 0);
});


let metadataHandlerSource;
async function eosFixture(emptyOrder = 'sync') {
  if (!metadataHandlerSource) {
    const code = await readFile(new URL('./components/VideoArtifact.tsx', import.meta.url), 'utf8');
    const ast = parse(code, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
    let handler;
    const visit = node => {
      if (!node || typeof node !== 'object') return;
      if (node.type === 'JSXAttribute' && node.name.name === 'onLoadedMetadata') handler = node.value.expression;
      for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
    };
    visit(ast); assert.ok(handler);
    metadataHandlerSource = code.slice(handler.start, handler.end);
  }
  class EosMedia extends Media {
    duration = 97.2666667; time = this.duration; presentedTime = this.duration; ended = true; eosLocked = true; loading = false; src = 'owned-fixture.mp4';
    pendingEmpty = false; listeners = new Map();
    addEventListener(name, listener, options) { const values = this.listeners.get(name) ?? new Set(); values.add(listener); this.listeners.set(name, values); super.addEventListener(name, listener, options); }
    removeEventListener(name, listener, options) { this.listeners.get(name)?.delete(listener); super.removeEventListener(name, listener, options); }
    get listenerCount() { return [...this.listeners.values()].reduce((n, values) => n + values.size, 0); }
    load() {
      this.sourceLoads++; this.loading = true; this.eosLocked = false;
      this.ended = false; this.time = 0; this.readyState = 0; this.seeking = false; this.paused = true;
      if (emptyOrder === 'sync') this.dispatchEvent(new Event('emptied')); else this.pendingEmpty = true;
    }
    metadata() { this.loading = false; this.readyState = 2; this.dispatchEvent(new Event('loadedmetadata')); }
    empty() { assert.equal(this.pendingEmpty, true); this.pendingEmpty = false; this.dispatchEvent(new Event('emptied')); }
    finishSeek(actual = this.time) { if (!this.eosLocked && !this.loading) { this.presentedTime = actual; super.finishSeek(actual); } }
  }
  const node = new EosMedia(), errors = [], positions = []; let source = 'owned-source', range = { startTicks: 2.5 * T, endTicks: Math.round(node.duration * T) };
  const context = { video: node, TICKS_PER_SECOND: T, range: () => range, setMediaDuration() {}, playback: undefined };
  const metadata = vm.runInNewContext(`(${metadataHandlerSource})`, context);
  // Match the real JSX registration before the onMount media adapter. Reload
  // metadata must not overwrite a user seek with the default trim beginning.
  node.addEventListener('loadedmetadata', metadata);
  const player = new VideoPlayback(node, { identity: () => source, range: () => range, position: value => positions.push(value), playing() {}, error: value => errors.push(value) });
  context.playback = player;
  return { node, player, errors, positions, get range() { return range; }, set range(value) { range = value; }, set source(value) { source = value; }, close() { player.dispose(); node.removeEventListener('loadedmetadata', metadata); } };
}

test('real ended media preparation reloads once before seek and the production metadata handler cannot replace a paused 19s target', async () => {
  for (const emptyOrder of ['sync', 'queued']) {
    const f = await eosFixture(emptyOrder), range = structuredClone(f.range), node = f.node, src = node.src;
    try {
      f.player.seek(19 * T); const flushed = f.player.seeks.flush(); const observed = flushed.catch(() => {}); await turn();
      assert.equal(node.playCalls, 0);
      if (node.readyState === 0) { f.node.metadata(); await turn(); }
      assert.deepEqual(node.assignments, [19], 'initial metadata must not insert an unrelated default seek');
      if (node.pendingEmpty) node.empty();
      node.finishSeek(); await flushed; await observed;
      assert.equal(node.presentedTime, 19, 'the actual presented frame must leave the old ending, not only its optimistic clock');
      assert.equal(node.currentTime, 19); assert.equal(node.seeking, false); assert.equal(node.paused, true);
      assert.equal(node.sourceLoads, 1);
      assert.equal(node.playCalls, 0); assert.equal(node.src, src); assert.equal(f.node, node);
      assert.deepEqual(f.range, range); assert.deepEqual(f.errors, []);
      f.player.seek(20 * T); await turn(); node.finishSeek(); await f.player.seeks.flush();
      assert.equal(node.sourceLoads, 1, 'ordinary seeks must not reload again');
    } finally { f.close(); await turn(); assert.equal(node.listenerCount, 0); }
  }
});

test('EOS replay preserves a nonzero trim beginning and delayed owned emptied cannot revoke the accepted new player', async () => {
  const f = await eosFixture('queued'); let pending;
  try {
    pending = f.player.play(); const rejected = pending.catch(() => {}); await turn();
    assert.equal(f.node.sourceLoads, 1); assert.equal(f.node.playCalls, 0);
    f.node.metadata(); await turn(); assert.deepEqual(f.node.assignments, [2.5]);
    assert.equal(f.node.playCalls, 0, 'no audio may start before the actual retained seek completes');
    f.node.finishSeek(); await pending; assert.equal(f.node.paused, false);
    f.node.empty(); await turn(); assert.equal(f.node.paused, false);
    assert.equal(f.node.playCalls, 1); assert.deepEqual(f.errors, []);
    // An unrelated later emptied still has the original cancellation contract.
    f.node.dispatchEvent(new Event('emptied')); assert.equal(f.node.paused, true);
    await rejected;
  } finally { f.close(); await pending?.catch(() => {}); }
});

test('EOS metadata preparation is canceled by pause, source, range or disposal and no late event restarts playback', async () => {
  for (const change of [f => f.player.pause(), f => { f.source = 'different'; f.player.invalidate(); }, f => { f.range = { startTicks: 3 * T, endTicks: 8 * T }; }, f => f.player.dispose()]) {
    const f = await eosFixture('queued'); const pending = f.player.play(), rejected = assert.rejects(pending, TrimCanceled);
    try {
      await turn(); assert.equal(f.node.sourceLoads, 1); change(f);
      f.node.metadata(); if (f.node.pendingEmpty) f.node.empty(); f.node.finishSeek();
      await rejected; await turn(); assert.equal(f.node.playCalls, 0); assert.equal(f.node.paused, true);
    } finally { f.close(); }
  }
});

test('EOS reset reports actual metadata failure without looping reload or hiding it as cancellation', async () => {
  const f = await eosFixture(); const pending = f.player.play(), rejected = assert.rejects(pending, /无法定位视频/);
  try {
    await turn(); f.node.error = { code: 3 }; f.node.dispatchEvent(new Event('error'));
    await rejected; f.node.metadata(); await turn();
    assert.equal(f.node.sourceLoads, 1); assert.equal(f.node.playCalls, 0);
  } finally { f.close(); }
});

test('reload metadata and actual seek share one eight-second absolute deadline rather than two budgets', async () => {
  const f = await eosFixture(), originalPerformance = globalThis.performance, originalTimeout = globalThis.setTimeout, delays = [];
  let now = 0, pending;
  globalThis.performance = { now: () => now };
  globalThis.setTimeout = (callback, delay, ...args) => { delays.push(delay); return originalTimeout(callback, delay, ...args); };
  try {
    pending = f.player.play(); const rejected = assert.rejects(pending, /无法定位视频/); await turn();
    now = 7000; f.node.metadata(); await turn();
    assert.ok(delays.includes(8000)); assert.ok(delays.includes(1000));
    assert.equal(f.node.playCalls, 0);
    now = 8001; f.node.finishSeek(); await rejected;
    assert.equal(f.node.playCalls, 0); assert.equal(f.node.sourceLoads, 1);
  } finally { f.close(); await pending?.catch(() => {}); globalThis.performance = originalPerformance; globalThis.setTimeout = originalTimeout; }
});

test('EOS replay never plays a seek acknowledged just before the retained trim beginning', async () => {
  const f = await eosFixture(); const pending = f.player.play(), rejected = assert.rejects(pending, /无法定位视频/);
  try {
    await turn(); f.node.metadata(); await turn();
    f.node.finishSeek(2.49); await rejected;
    assert.equal(f.node.playCalls, 0); assert.equal(f.node.paused, true);
  } finally { f.close(); }
});

test('an EOS annotation interval is established after metadata and still waits for raster readiness and owns cancellation', async () => {
  const f = await eosFixture(); let ready = false;
  const interval = { startTicks: 25_300_000, endTicks: 26_300_000 };
  const pending = f.player.playInterval(interval, () => true, () => ready);
  try {
    await turn(); f.node.metadata(); await turn(); assert.equal(f.node.assignments.at(-1), 2.53);
    f.node.finishSeek(); await turn(); assert.equal(f.node.playCalls, 0);
    ready = true; await pending; assert.equal(f.node.paused, false);
    const hold = new VideoAnnotationPlaybackHold();
    const update = loading => hold.update({ source: 'document', intent: f.player.playIntent(), loading, failed: false, playing: !f.node.paused, allowed: true }, () => f.player.holdForRaster(), () => f.player.resumeCurrent(), error => f.errors.push(error));
    update(true); assert.equal(f.node.paused, true);
    f.player.pause(); update(false); await turn(); assert.equal(f.node.paused, true);
    assert.deepEqual(f.errors, []);
  } finally { f.close(); await pending.catch(() => {}); }
});
