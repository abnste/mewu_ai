// Actual App.addRegion and lifecycle barrier; synthetic bridge/state only.
// node --experimental-vm-modules --test apps/desktop/src/region-addition.test.mjs
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';

const plain = value => JSON.parse(JSON.stringify(value));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const tick = () => new Promise(resolve => setImmediate(resolve));
const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const transform = value => stripTypeScriptTypes(value, { mode: 'transform' });
const source = await raw('./App.tsx');
const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const declarations = [];
function walk(value) {
  if (!value || typeof value !== 'object') return;
  if (value.type === 'FunctionDeclaration' && value.id?.name === 'addRegion') declarations.push(value);
  for (const child of Object.values(value)) {
    if (Array.isArray(child)) child.forEach(walk);
    else if (child && typeof child === 'object' && child.type) walk(child);
  }
}
walk(ast);
assert.equal(declarations.length, 1, 'exercise the unique production App.addRegion declaration');
const handler = source.slice(declarations[0].start, declarations[0].end);
const region = id => ({ id, x: 10, y: 20, width: 100, height: 80 });
const ref = id => ({ kind: 'region', id });

async function fixture() {
  const context = createContext({ console, setTimeout, clearTimeout });
  const load = async file => {
    const module = new SourceTextModule(transform(await raw(file)), { context });
    await module.link(() => { throw Error(`Unexpected runtime import in ${file}`); });
    await module.evaluate(); return module.namespace;
  };
  const { SpaceOperations } = await load('./exit-preparation.ts');
  const { PublishedRevision } = await load('./published-revision.ts');
  const state = { snapshot: { revision: 1, activeSceneId: 'a', scenes: [
    { id: 'a', closed: false, frozen: false, background: { id: 'background' }, refs: [ref('base')], regions: [] },
  ] }, drafts: {}, exiting: false, scrolling: false, flush: () => Promise.resolve() };
  const calls = [], legacy = [], flushes = [], operations = new SpaceOperations();
  const published = new PublishedRevision(() => state.snapshot.revision, async () => { throw Error('Unexpected full snapshot query'); }, 1000);
  const bridge = new Proxy({}, { get(_, name) {
    if (name !== 'addCaptureRegion') return () => { legacy.push(name); throw Error(`Forbidden legacy bridge path: ${String(name)}`); };
    return (...args) => { const gate = deferred(); calls.push({ args: plain(args), gate }); return gate.promise; };
  } });
  const bindings = { bridge, snapshot: () => state.snapshot,
    exitPreparing: () => state.exiting, scroll: () => state.scrolling,
    referenceDrafts: () => state.drafts, setReferenceDrafts: update => { state.drafts = update(state.drafts); },
    regionGeometry: { flush: () => { flushes.push(true); return state.flush(); } },
    spaceOperations: operations, publishedRevision: published };
  const imports = new SyntheticModule(Object.keys(bindings), function () {
    for (const [key, value] of Object.entries(bindings)) this.setExport(key, value);
  }, { context });
  const owner = new SourceTextModule(transform(`
    import {${Object.keys(bindings).join(',')}} from 'fixture';
    let commands=Promise.resolve(),exitFailure,disposed=false;
    ${handler}
    export {addRegion};
    export const queueBefore=value=>{commands=value;};
    export const drained=()=>commands;
    export const dispose=()=>{disposed=true;};
    export const failure=()=>exitFailure;
  `), { context });
  await owner.link(() => imports); await owner.evaluate();
  const commit = (index, revision = state.snapshot.revision + 1) => {
    const [sceneId, , value, refs] = calls[index].args;
    state.snapshot = { ...state.snapshot, revision, scenes: state.snapshot.scenes.map(scene => scene.id === sceneId
      ? { ...scene, regions: [...scene.regions, plain(value)], refs: plain(refs) } : scene) };
    published.changed(); return revision;
  };
  const drain = async () => { await operations.drain(() => true); await owner.namespace.drained(); };
  return { state, calls, legacy, flushes, operations, published, commit, drain,
    ...owner.namespace, close: () => published.dispose() };
}

test('consecutive additions serialize one atomic bridge call each and read live references at execution', async () => {
  const f = await fixture();
  try {
    const prior = deferred(); f.queueBefore(prior.promise);
    const first = f.addRegion('a', region('one')), second = f.addRegion('a', region('two'));
    await tick(); assert.equal(f.calls.length, 0);
    const edited = [ref('edited-before-execution')]; f.state.drafts = { a: edited };
    let drained = false; const barrier = f.operations.drain(() => true).then(() => { drained = true; });
    prior.resolve(); await tick();
    assert.equal(f.calls.length, 1); assert.equal(drained, false);
    assert.deepEqual(f.calls[0].args, ['a', 'background', region('one'), [...edited, ref('one')]]);
    const revision = f.commit(0); // publication before IPC reply
    await tick(); assert.equal(f.calls.length, 1, 'publication alone must not release the pending bridge call');
    f.calls[0].gate.resolve(revision); await first; await tick();
    assert.equal(f.calls.length, 2);
    assert.deepEqual(f.calls[1].args[3], [...edited, ref('one'), ref('two')]);
    f.calls[1].gate.resolve(revision + 1); // IPC reply before publication
    await tick(); assert.equal(drained, false);
    f.commit(1, revision + 1); await second; await barrier;
    assert.equal(drained, true); assert.deepEqual(f.legacy, []);
    assert.deepEqual(plain(f.state.drafts.a), [...edited, ref('one'), ref('two')]);
  } finally { f.close(); }
});

test('without a local reference draft the next addition uses the preceding authoritative commit', async () => {
  const f = await fixture();
  try {
    const first = f.addRegion('a', region('one')); await tick();
    f.calls[0].gate.resolve(f.commit(0)); await first;
    f.state.drafts = {}; // the real draft persistence may remove its clean overlay
    const second = f.addRegion('a', region('two')); await tick();
    assert.deepEqual(f.calls[1].args[3], [ref('base'), ref('one'), ref('two')]);
    f.calls[1].gate.resolve(f.commit(1)); await second; await f.drain();
    assert.deepEqual(f.legacy, []);
  } finally { f.close(); }
});

for (const [name, mutate] of [
  ['background replacement', f => { f.state.snapshot.scenes[0].background.id = 'replacement'; }],
  ['active scene replacement', f => { f.state.snapshot.activeSceneId = 'other'; }],
  ['closed scene', f => { f.state.snapshot.scenes[0].closed = true; }],
  ['frozen scene', f => { f.state.snapshot.scenes[0].frozen = true; }],
  ['exit preparation', f => { f.state.exiting = true; }],
  ['scroll capture', f => { f.state.scrolling = true; }],
  ['disposed owner', f => { f.dispose(); }],
]) test(`${name} before dispatch rejects without a native call and releases its operation`, async () => {
  const f = await fixture();
  try {
    const prior = deferred(); f.queueBefore(prior.promise);
    const result = f.addRegion('a', region('one'));
    const rejected = assert.rejects(result, /截图已变化/);
    await tick(); mutate(f); prior.resolve(); await rejected; await f.drain();
    assert.equal(f.calls.length, 0); assert.deepEqual(f.legacy, []);
  } finally { f.close(); }
});

test('already exiting or missing screenshot rejects before acquiring an operation', async () => {
  const f = await fixture();
  try {
    f.state.exiting = true; await assert.rejects(f.addRegion('a', region('one')), /正在退出/);
    f.state.exiting = false; delete f.state.snapshot.scenes[0].background;
    await assert.rejects(f.addRegion('a', region('two')), /截图已变化/);
    await f.drain(); assert.equal(f.flushes.length, 0); assert.equal(f.calls.length, 0);
  } finally { f.close(); }
});

test('reference editing during native save keeps the new draft ownership for subsequent additions', async () => {
  const f = await fixture();
  try {
    const original = [ref('original-draft')]; f.state.drafts = { a: original };
    const first = f.addRegion('a', region('one')); await tick();
    const explicit = [ref('user-changed-while-saving')]; f.state.drafts = { a: explicit };
    f.calls[0].gate.resolve(f.commit(0)); await first;
    assert.equal(f.state.drafts.a, explicit, 'late save must not overwrite a newer reference array');
    const second = f.addRegion('a', region('two')); await tick();
    assert.deepEqual(f.calls[1].args[3], [...explicit, ref('two')]);
    f.calls[1].gate.resolve(f.commit(1)); await second; await f.drain();
  } finally { f.close(); }
});

test('background changes while a native save is in flight cannot write a draft into the replacement', async () => {
  const f = await fixture();
  try {
    const first = f.addRegion('a', region('one')); await tick();
    f.state.snapshot.scenes[0].background.id = 'replacement';
    f.calls[0].gate.resolve(undefined); await first; await f.drain();
    assert.deepEqual(f.state.drafts, {});
    assert.equal(f.calls.length, 1, 'an already dispatched operation is not retried against new pixels');
  } finally { f.close(); }
});

test('a failed native save releases its barrier and does not poison later queued additions', async () => {
  const f = await fixture();
  try {
    const first = f.addRegion('a', region('one')), rejected = assert.rejects(first, /disk failure/);
    const second = f.addRegion('a', region('two')); await tick();
    assert.equal(f.calls.length, 1); f.calls[0].gate.reject(Error('disk failure'));
    await rejected; await tick(); assert.equal(f.calls.length, 2);
    assert.deepEqual(f.calls[1].args[3], [ref('base'), ref('two')], 'failed additions do not invent references');
    f.calls[1].gate.resolve(f.commit(1)); await second; await f.drain();
    assert.equal(f.failure(), undefined); assert.deepEqual(f.legacy, []);
  } finally { f.close(); }
});

test('geometry flush rejection also releases its operation and admits a later successful save', async () => {
  const f = await fixture();
  try {
    f.state.flush = async () => { throw Error('geometry conflict'); };
    await assert.rejects(f.addRegion('a', region('one')), /geometry conflict/);
    await f.drain(); assert.equal(f.calls.length, 0);
    f.state.flush = async () => {};
    const next = f.addRegion('a', region('two')); await tick();
    f.calls[0].gate.resolve(f.commit(0)); await next; await f.drain();
    assert.deepEqual(f.calls[0].args[3], [ref('base'), ref('two')]);
  } finally { f.close(); }
});

test('a save failure after exit preparation begins is retained for the real exit barrier', async () => {
  const f = await fixture();
  try {
    const failure = Error('unacknowledged save during exit');
    const result = f.addRegion('a', region('one'));
    const rejected = assert.rejects(result, error => error === failure);
    await tick(); f.state.exiting = true; f.calls[0].gate.reject(failure);
    await rejected; await f.drain();
    assert.equal(f.failure(), failure); assert.deepEqual(f.state.drafts, {});
    assert.equal(f.calls.length, 1);
  } finally { f.close(); }
});
