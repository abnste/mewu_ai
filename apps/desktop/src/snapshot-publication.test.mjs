// Actual bridge subscription/read code and PublishedRevision; synthetic IPC only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { webcrypto } from 'node:crypto';
import { loadProviderPresets, loadGeometryHistory, loadAssetImport, loadHighlightSource } from './geometry-test-module.mjs';

const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const ts = text => stripTypeScriptTypes(text, { mode: 'transform' });
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const snapshot = (revision, tag = '') => ({ schemaVersion: 1, revision, activeSceneId: 'active',
  scenes: [{ id: 'active', regions: [], items: [], messages: [], draft: tag },
    { id: 'historical', regions: [], items: [], messages: [{ text: 'synthetic preserved history' }] }],
  agents: [], connection: { baseUrl: '', model: '', hasKey: false } });
async function fixture(failEvent) {
  const listeners = new Set(), reads = [], timers = new Map(), measurements = [];
  let timerId = 0;
  const context = createContext({ crypto: webcrypto, structuredClone, Date, Map, Set, URL, Blob,
    setTimeout: (callback, delay) => { const id = ++timerId; timers.set(id, { callback, delay }); return id; },
    clearTimeout: id => timers.delete(id) });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const core = synthetic({ isTauri: () => true, invoke: command => {
    if (command === 'runtime_info') return Promise.resolve({ platform: 'windows', hotkey: 'Alt+A', captureBackend: 'fixture', contentOrigin: 'http://127.0.0.1:49000', measureInteractions: true });
    assert.equal(command, 'get_snapshot'); const item = deferred(); reads.push(item); return item.promise;
  } });
  const events = synthetic({ listen: async (name, callback, options) => {
    if (name === failEvent) throw Error('listen failed');
    const entry = { name, callback, target: options?.target }; listeners.add(entry); return () => listeners.delete(entry);
  } });
  const performance = synthetic({ configureCapturePerformance: () => {}, captureSnapshotReceived: value => { measurements.push(['received', value.revision]); return {}; },
    captureSnapshotNormalized: () => measurements.push(['normalized']) });
  const policy = new SourceTextModule(ts(await raw('connection-policy.ts')), { context }); await policy.link(() => { throw Error('unexpected dependency'); });
  const history = await loadGeometryHistory(context), imported = await loadAssetImport(context);
  const mosaic = new SourceTextModule(ts(await raw('mosaic-preview.ts')), { context }); await mosaic.link(() => { throw Error('unexpected dependency'); });
  const bridge = new SourceTextModule(ts(await raw('bridge.ts')), { context, initializeImportMeta: meta => { meta.glob = () => ({}); } });
  await bridge.link(async name => name === './highlight-source' ? await loadHighlightSource(context)
    : name === './provider-presets' ? await loadProviderPresets(context) : name === './asset-import' ? imported
    : name === './region-geometry-history' ? history : name === './connection-policy' ? policy
    : name === './mosaic-preview' ? mosaic : name === './capture-performance' ? performance
    : name.endsWith('/core') ? core : events);
  await bridge.evaluate();
  const revision = new SourceTextModule(ts(await raw('published-revision.ts')), { context });
  await revision.link(() => { throw Error('unexpected witness dependency'); }); await revision.evaluate();
  return { api: bridge.namespace, PublishedRevision: revision.namespace.PublishedRevision, reads, listeners, timers, measurements,
    emit(target, payload, name = 'snapshot-revision') { for (const item of [...listeners]) if (item.name === name && item.target === target) item.callback({ payload }); },
    async fireTimer(delay) { const entry = [...timers].find(([, value]) => value.delay === delay); assert.ok(entry, 'expected bounded timer'); timers.delete(entry[0]); entry[1].callback(); await tick(); } };
}

test('notice and explicit full read share one actual invoke and preserve complete history', async () => {
  const f = await fixture(), seen = [];
  const stop = await f.api.subscribe(value => seen.push(value), () => {}, error => { throw Error(error); });
  f.emit('space', { revision: 2 }); const explicit = f.api.getSnapshot(); await tick();
  assert.equal(f.reads.length, 1); f.reads[0].resolve(snapshot(2));
  const full = await explicit; await tick();
  assert.equal(seen.length, 1); assert.equal(full, seen[0]);
  assert.equal(full.scenes[1].messages[0].text, 'synthetic preserved history');
  assert.deepEqual(f.measurements, [['received', 2], ['normalized']]); stop();
});

test('new revisions coalesce to one pending number and full snapshots make monotonic progress', async () => {
  const f = await fixture(), seen = [], errors = [];
  const stop = await f.api.subscribe(value => seen.push(value.revision), () => {}, value => errors.push(value));
  f.emit('space', { revision: 2 }); await tick();
  for (let revision = 3; revision <= 100; revision++) f.emit('space', { revision });
  assert.equal(f.reads.length, 1); f.reads[0].resolve(snapshot(2)); await tick();
  assert.equal(f.reads.length, 2); assert.deepEqual(seen, [2]);
  f.reads[1].resolve(snapshot(100)); await tick();
  assert.deepEqual(seen, [2, 100]); assert.deepEqual(errors, []); assert.equal(f.timers.size, 0); stop();
});

test('duplicate/older notices dedupe only invalidations while an equal-revision explicit recovery still reads', async () => {
  const f = await fixture(), seen = [];
  const stop = await f.api.subscribe(value => seen.push(value), () => {}, () => {});
  f.emit('space', { revision: 5 }); await tick(); f.reads[0].resolve(snapshot(5)); await tick();
  for (const value of [{ revision: 5 }, { revision: 4 }, { revision: 0 }, { revision: -1 }, { revision: 5, extra: 1 }, { revision: NaN }, 5, null, { revision: Number.MAX_SAFE_INTEGER + 1 }]) f.emit('space', value);
  await tick(); assert.equal(f.reads.length, 1); assert.equal(seen.length, 1);
  const recovered = f.api.getSnapshot(); await tick(); assert.equal(f.reads.length, 2);
  f.reads[1].resolve(snapshot(5, 'equal-revision recovery')); assert.equal((await recovered).scenes[0].draft, 'equal-revision recovery'); stop();
});

test('missing notice keeps ordinary getSnapshot recovery and concurrent callers share only the live flight', async () => {
  const f = await fixture(), first = f.api.getSnapshot(), second = f.api.getSnapshot();
  await tick(); assert.equal(f.reads.length, 1); f.reads[0].resolve(snapshot(7));
  assert.equal(await first, await second); assert.equal(f.measurements.length, 1); // normalized hook is advisory; no space consumer.
  const next = f.api.getSnapshot(); await tick(); assert.equal(f.reads.length, 2); f.reads[1].resolve(snapshot(7)); await next;
});

test('a failed read reports and retries once, succeeds later, and never leaves a rejected shared cache', async () => {
  const f = await fixture(), seen = [], errors = [];
  const stop = await f.api.subscribe(value => seen.push(value.revision), () => {}, value => errors.push(value));
  f.emit('space', { revision: 3 }); await tick(); f.reads[0].reject(Error('synthetic read failure')); await tick();
  assert.equal(errors.length, 1); assert.equal(f.timers.size, 1);
  await f.fireTimer(50); assert.equal(f.reads.length, 2); f.reads[1].resolve(snapshot(3)); await tick();
  assert.deepEqual(seen, [3]); assert.equal(f.timers.size, 0);
  const explicit = f.api.getSnapshot(); await tick(); assert.equal(f.reads.length, 3); f.reads[2].resolve(snapshot(3)); await explicit; stop();
});

test('repeated failure is bounded and a subsequent new publication can recover', async () => {
  const f = await fixture(), seen = [], errors = [];
  const stop = await f.api.subscribe(value => seen.push(value.revision), () => {}, value => errors.push(value));
  f.emit('space', { revision: 3 }); await tick(); f.reads[0].reject(Error('first')); await tick();
  await f.fireTimer(50); f.reads[1].reject(Error('second')); await tick();
  assert.equal(f.reads.length, 2); assert.equal(f.timers.size, 0); assert.equal(errors.length, 2);
  f.emit('space', { revision: 4 }); await tick(); assert.equal(f.reads.length, 3); f.reads[2].resolve(snapshot(4)); await tick();
  assert.deepEqual(seen, [4]); stop();
});

test('a persistently stale host cannot create an unbounded full-query loop', async () => {
  const f = await fixture(), errors = [];
  const stop = await f.api.subscribe(() => { throw Error('stale data delivered'); }, () => {}, value => errors.push(value));
  f.emit('space', { revision: 10 }); await tick(); f.reads[0].resolve(snapshot(0)); await tick();
  f.reads[1].resolve(snapshot(0)); await tick(); assert.equal(f.timers.size, 1);
  await f.fireTimer(50); f.reads[2].resolve(snapshot(0)); await tick(); f.reads[3].resolve(snapshot(0)); await tick();
  assert.equal(f.reads.length, 4); assert.equal(f.timers.size, 0); assert.equal(errors.length, 2); stop();
});

test('dispose cancels retry resources and ignores both late success and late rejection', async () => {
  for (const reject of [false, true]) {
    const f = await fixture(), seen = [], errors = [];
    const stop = await f.api.subscribe(value => seen.push(value), () => {}, value => errors.push(value));
    f.emit('space', { revision: 6 }); await tick(); stop(); stop();
    if (reject) f.reads[0].reject(Error('late')); else f.reads[0].resolve(snapshot(6)); await tick();
    assert.deepEqual(seen, []); assert.deepEqual(errors, []); assert.equal(f.listeners.size, 0); assert.equal(f.timers.size, 0);
  }
  const f = await fixture(), stop = await f.api.subscribe(() => {}, () => {}, () => {});
  f.emit('space', { revision: 6 }); await tick(); f.reads[0].reject(Error('retry')); await tick(); stop(); assert.equal(f.timers.size, 0);
});

test('space and settings notices retain exact target routing and share no delivery to a foreign surface', async () => {
  const f = await fixture(), space = [], settings = [];
  const stopSpace = await f.api.subscribe(value => space.push(value.revision), () => {}, () => {});
  const stopSettings = await f.api.subscribe(value => settings.push(value.revision), () => {}, () => {}, 'settings');
  f.emit('other', { revision: 100 }); await tick(); assert.equal(f.reads.length, 0);
  f.emit('space', { revision: 2 }); await tick(); f.reads[0].resolve(snapshot(2)); await tick();
  assert.deepEqual(space, [2]); assert.deepEqual(settings, []);
  f.emit('settings', { revision: 3 }); await tick(); f.reads[1].resolve(snapshot(3)); await tick();
  assert.deepEqual(space, [2]); assert.deepEqual(settings, [3]); stopSpace(); stopSettings();
});

test('partial native listener registration failure disposes pump and diagnostics consumer', async () => {
  for (const name of ['snapshot-revision', 'run-event', 'host-error']) {
    const f = await fixture(name);
    await assert.rejects(() => f.api.subscribe(() => {}, () => {}, () => {}), /listen failed/);
    assert.equal(f.listeners.size, 0); assert.equal(f.timers.size, 0);
    const explicit = f.api.getSnapshot(); await tick(); f.reads[0].resolve(snapshot(1)); await explicit;
    assert.equal(f.measurements.some(value => value[0] === 'received'), false);
  }
});

test('PublishedRevision and drawing reply witness reuse the notice full-read flight during fallback', async () => {
  const f = await fixture(); let current;
  const barrier = new f.PublishedRevision(() => current?.revision, async () => { current = await f.api.getSnapshot(); barrier.changed(); });
  const stop = await f.api.subscribe(value => { current = value; barrier.changed(); }, () => {}, () => {});
  const waiting = barrier.wait(12, () => current?.scenes[0].draft === 'persisted drawing');
  f.emit('space', { revision: 12 }); await tick(); assert.equal(f.reads.length, 1);
  await f.fireTimer(1000); assert.equal(f.reads.length, 1, 'timeout refresh must join active notice request');
  f.reads[0].resolve(snapshot(12, 'persisted drawing')); await waiting; await tick();
  assert.equal(f.reads.length, 1); assert.equal(f.timers.size, 0); stop(); barrier.dispose();
});

test('event-before-reply satisfies PublishedRevision immediately and missing event uses ordinary full query', async () => {
  const f = await fixture(); let current;
  const barrier = new f.PublishedRevision(() => current?.revision, async () => { current = await f.api.getSnapshot(); barrier.changed(); });
  const stop = await f.api.subscribe(value => { current = value; barrier.changed(); }, () => {}, () => {});
  f.emit('space', { revision: 4 }); await tick(); f.reads[0].resolve(snapshot(4)); await tick();
  await barrier.wait(4); assert.equal(f.reads.length, 1); assert.equal(f.timers.size, 0);
  const waiting = barrier.wait(5); await f.fireTimer(1000); assert.equal(f.reads.length, 2);
  f.reads[1].resolve(snapshot(5)); await waiting; stop(); barrier.dispose();
});

test('continuous newer notices cannot starve full-state progress and disposal stops catch-up', async () => {
  const f = await fixture(), seen = [], errors = [];
  const stop = await f.api.subscribe(value => seen.push(value.revision), () => {}, value => errors.push(value));
  f.emit('space', { revision: 2 }); await tick();
  for (let read = 0; read < 6; read++) {
    f.emit('space', { revision: read + 3 });
    f.reads[read].resolve(snapshot(read + 2)); await tick();
    assert.equal(seen.at(-1), read + 2, 'every increasing authoritative read must be delivered despite a newer notice');
    assert.equal(f.reads.length, read + 2, 'only one catch-up request may be in flight');
  }
  assert.deepEqual(seen, [2, 3, 4, 5, 6, 7]); assert.deepEqual(errors, []);
  stop(); f.reads[6].resolve(snapshot(8)); await tick();
  assert.equal(f.reads.length, 7); assert.deepEqual(seen, [2, 3, 4, 5, 6, 7]); assert.equal(f.timers.size, 0);
});

test('a later explicit recovery joins one fresh query when an older flight predates its known notice', async () => {
  const f = await fixture(), seen = [];
  const stop = await f.api.subscribe(value => seen.push(value.revision), () => {}, () => {});
  const original = f.api.getSnapshot(); await tick(); assert.equal(f.reads.length, 1);
  f.emit('space', { revision: 9 }); const recovery = f.api.getSnapshot(); await tick();
  assert.equal(f.reads.length, 1); f.reads[0].resolve(snapshot(8));
  assert.equal((await original).revision, 8); await tick();
  assert.equal(f.reads.length, 2, 'notice pump and later recovery must share the fresh follow-up');
  f.reads[1].resolve(snapshot(9)); assert.equal((await recovery).revision, 9); await tick();
  assert.deepEqual(seen, [9]); stop();
});
