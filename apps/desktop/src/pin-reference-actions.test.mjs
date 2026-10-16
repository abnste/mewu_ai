// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';

const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const nodes = [];
const walk = node => {
  if (!node || typeof node !== 'object') return;
  if (node.type) nodes.push(node);
  for (const [key, value] of Object.entries(node)) {
    if (key === 'loc') continue;
    if (Array.isArray(value)) value.forEach(walk);
    else if (value && typeof value === 'object') walk(value);
  }
};
walk(ast);
const declarations = ['pinItem', 'pinReferenced', 'canvasScene'].map(name => {
  const node = nodes.find(value => value.type === 'VariableDeclarator' && value.id.name === name);
  assert.ok(node, `Real App.${name} is required`);
  return `const ${source.slice(node.start, node.end)};`;
});
const reference = nodes.find(value => value.type === 'FunctionDeclaration' && value.id.name === 'referencePin');
assert.ok(reference, 'Real App.referencePin is required');
const code = stripTypeScriptTypes([...declarations, source.slice(reference.start, reference.end), '({pinItem,pinReferenced,referencePin,canvasScene})'].join('\n'), { mode: 'transform' });
const blackboardSource = await readFile(new URL('./blackboard.ts', import.meta.url), 'utf8');
const { isBlackboard } = await import('data:text/javascript;base64,' + Buffer.from(stripTypeScriptTypes(blackboardSource, { mode: 'transform' })).toString('base64'));
const plain = value => JSON.parse(JSON.stringify(value));
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const oldObject = Object.freeze({ id: 'pin-id', originAssetId: 'origin-asset', attachmentAssetId: 'previous-scene-baked' });
const liveObject = Object.freeze({ ...oldObject, attachmentAssetId: 'new-baked-asset' });
const image = (id, assetId) => ({ id, asset: { id: assetId, kind: 'image' } });
const destination = () => ({ id: 'target-scene', items: [] });
const referencedScene = () => ({ id: 'target-scene', items: [image('new-reference-item', liveObject.attachmentAssetId)] });
const snapshot = () => ({ activeSceneId: 'target-scene', scenes: [referencedScene()] });

function fixture(options = {}) {
  let current = options.scene ?? destination(), busy = false, exiting = false, pins = [oldObject], references = [];
  const calls = [], insertions = [], accepted = [], cleared = [], toggles = [], errors = [], events = [];
  const context = vm.createContext({
    scene: () => current, pinObjects: () => pins, refs: () => references,
    busy: () => busy, sending: () => false, exitPreparing: () => exiting,
    setBusy: value => { busy = value; },
    flush: async () => { events.push('flush'); await options.flush?.(); },
    referencePinObject: async (id, sceneId) => {
      calls.push({ id, sceneId }); events.push('native');
      return options.native ? options.native() : snapshot();
    },
    clearReferenceDraft: sceneId => { cleared.push(sceneId); },
    accept: next => {
      accepted.push(plain(next)); events.push('accept');
      current = next.scenes.find(scene => scene.id === next.activeSceneId);
    },
    pinObjectsSubscription: { refresh: async () => {
      events.push('refresh'); await options.refresh?.();
      pins = options.livePins ?? [liveObject]; events.push('refreshed');
    } },
    toggleReference: value => toggles.push(plain(value)),
    setReferenceInsertion: value => { insertions.push(plain(value)); events.push('insert'); },
    referenceSerial: 0, showError: cause => errors.push(cause), isBlackboard,
  });
  const api = vm.runInContext(code, context);
  return { api, calls, insertions, accepted, cleared, toggles, errors, events,
    current: () => current, busy: () => busy,
    setScene: value => { current = value; }, setExiting: value => { exiting = value; },
    setPins: value => { pins = value; }, setRefs: value => { references = value; },
  };
}

test('pin reference waits for the new baked identity before inserting one @ mention', async () => {
  const flush = deferred(), native = deferred(), refresh = deferred();
  const f = fixture({ flush: () => flush.promise, native: () => native.promise, refresh: () => refresh.promise });
  const operation = f.api.referencePin(oldObject);
  assert.equal(f.busy(), true);
  assert.equal(f.calls.length, 0); assert.equal(f.insertions.length, 0);
  flush.resolve(); await tick();
  assert.deepEqual(f.calls, [{ id: oldObject.id, sceneId: 'target-scene' }]);
  native.resolve(snapshot()); await tick();
  assert.equal(f.accepted.length, 1); assert.equal(f.insertions.length, 0, 'A stale object cannot create the mention while the registry is refreshing');
  refresh.resolve(); await operation;
  assert.deepEqual(f.insertions, [{ sceneId: 'target-scene', reference: { kind: 'item', id: 'new-reference-item' }, serial: 1 }]);
  assert.ok(f.events.indexOf('refreshed') < f.events.indexOf('insert'));
  assert.equal(oldObject.attachmentAssetId, 'previous-scene-baked');
  assert.equal(oldObject.originAssetId, 'origin-asset');
  assert.equal(f.busy(), false); assert.deepEqual(f.errors, []);
});

test('pin referenced state resolves the current baked item and toggles it without another native copy', async () => {
  const f = fixture({ scene: referencedScene() });
  f.setPins([liveObject]); f.setRefs([{ kind: 'item', id: 'new-reference-item' }]);
  assert.equal(f.api.pinItem(liveObject).id, 'new-reference-item');
  assert.equal(f.api.pinReferenced(liveObject), true);
  assert.equal(f.api.pinReferenced(oldObject), false, 'The old baked id from another scene is not the current reference');
  await f.api.referencePin(liveObject);
  assert.deepEqual(f.toggles, [{ kind: 'item', id: 'new-reference-item' }]);
  assert.equal(f.calls.length, 0); assert.equal(f.insertions.length, 0); assert.equal(f.accepted.length, 0);
});

test('capture presentation hides original and baked pin identities while blackboard media stay editable', () => {
  const original = image('original-item', 'origin-asset'), baked = image('baked-item', 'new-baked-asset'), other = image('other-item', 'ordinary-asset');
  const current = { id: 'target-scene', items: [original, baked, other] }, before = structuredClone(current);
  const f = fixture({ scene: current }); f.setPins([liveObject]);
  assert.deepEqual(Array.from(f.api.canvasScene(current).items, item => item.id), ['other-item']);
  assert.deepEqual(current, before, 'Presentation must not delete stored scene items');
  const board = { ...current, background: { id: 'board-pixels', kind: 'image', name: '黑板.png', path: 'D:/synthetic/board-pixels.board.png' } };
  assert.equal(f.api.canvasScene(board), board, 'The original blackboard scene object and its editable media must be preserved');
  const legacy = fixture({ scene: current }); legacy.setPins([{ ...liveObject, originAssetId: undefined }]);
  assert.deepEqual(Array.from(legacy.api.canvasScene(current).items, item => item.id), ['original-item', 'other-item']);
});

for (const stage of ['flush', 'native', 'refresh']) {
  for (const change of ['scene', 'exit']) {
    test(`${change} during awaited ${stage} cannot insert a pin mention into a successor scene`, async () => {
      const pause = deferred();
      const options = stage === 'flush' ? { flush: () => pause.promise }
        : stage === 'native' ? { native: async () => { await pause.promise; return snapshot(); } }
          : { refresh: () => pause.promise };
      const f = fixture(options), operation = f.api.referencePin(oldObject);
      await tick();
      if (change === 'scene') f.setScene({ id: 'successor-scene', items: [image('successor-item', 'new-baked-asset')] });
      else f.setExiting(true);
      pause.resolve(); await operation;
      assert.equal(f.insertions.length, 0);
      assert.equal(f.busy(), false);
      if (stage !== 'refresh') { assert.equal(f.accepted.length, 0); assert.equal(f.cleared.length, 0); }
      if (stage === 'flush') assert.equal(f.calls.length, 0);
      if (change === 'scene') assert.equal(f.current().id, 'successor-scene', 'An obsolete native result must not switch the user back');
    });
  }
}

for (const stage of ['flush', 'native', 'refresh']) {
  test(`${stage} failure never fabricates a successful pin mention`, async () => {
    const error = Error(`synthetic ${stage} failure`);
    const f = fixture({ [stage]: async () => { throw error; } });
    await f.api.referencePin(oldObject);
    assert.equal(f.insertions.length, 0); assert.equal(f.busy(), false);
    assert.equal(f.errors.length, 1); assert.equal(f.errors[0], error);
    if (stage !== 'refresh') assert.equal(f.accepted.length, 0);
  });
}

test('a missing native snapshot or missing refreshed object cannot invent a pin reference', async () => {
  const missingSnapshot = fixture({ native: async () => null });
  await missingSnapshot.api.referencePin(oldObject);
  assert.equal(missingSnapshot.insertions.length, 0); assert.equal(missingSnapshot.accepted.length, 0); assert.equal(missingSnapshot.busy(), false);
  const missingObject = fixture({ livePins: [] });
  await missingObject.api.referencePin(oldObject);
  assert.equal(missingObject.insertions.length, 0); assert.equal(missingObject.busy(), false);
});
