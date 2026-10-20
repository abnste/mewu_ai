// SPDX-License-Identifier: MPL-2.0
// Exercise the production authoring readers; no renderer, native IPC or data.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';

const { DrawingFlushRegistry } = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL('./drawing-flush.ts', import.meta.url), 'utf8'), { mode: 'transform' })).toString('base64')}`);
async function reader(file, object, property) {
  const source = await readFile(new URL(file, import.meta.url), 'utf8');
  const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  let found;
  function visit(node) {
    if (!node || typeof node !== 'object') return;
    if (object === null && ['FunctionDeclaration', 'ClassDeclaration'].includes(node.type) && node.id?.name === property) found = node;
    if (node.type === 'AssignmentExpression' && node.left.type === 'MemberExpression'
      && node.left.object.name === object && node.left.property.name === property) found = node.right;
    for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
  }
  visit(ast); assert.ok(found, `${file} ${object ?? ''}.${property}`);
  return stripTypeScriptTypes(`(${source.slice(found.start, found.end)})`, { mode: 'transform' });
}
const sharedReader = await reader('./components/SharedDrawingEditor.tsx', null, 'controlInteractionActive');
const objectReader = await reader('./components/ArtifactCard.tsx', 'objectFlush', 'interacting');
const objectReaders = await reader('./components/SpaceCanvas.tsx', 'flushObjects', 'interacting');
const textReader = await reader('./components/BlackboardText.tsx', 'textFlush', 'interacting');
const VideoFlushRegistry = vm.runInNewContext(await reader('./video-requests.ts', null, 'VideoFlushRegistry'), { TrimCanceled: Error });
function shared() {
  const values = {};
  const scope = { disposed: false, cancelGesture: undefined, flight: undefined, eraseFlight: undefined, tableCopy: undefined, props: { busy: true },
    strokeQueue: { pending: () => false }, historyLane: { pending: () => false } };
  for (const name of ['edit', 'draft', 'marquee', 'storedDraft', 'erasing', 'rasterPreview', 'selectionReading', 'finishing', 'imageZooming', 'objectDragging', 'pending', 'historyPending']) scope[name] = () => values[name];
  scope.groupDraft = () => values.groupDraft ?? [];
  const active = vm.runInNewContext(sharedReader, scope);
  return { active, values, scope };
}

test('the current flush owner must affirm idle for the exact scene and background, with unknowns failing closed', () => {
  const registry = new DrawingFlushRegistry();
  assert.equal(registry.idle('scene', 'background'), false);
  const flush = async () => {};
  const old = registry.register(flush);
  assert.equal(registry.idle('scene', 'background'), false); assert.equal(registry.interacting(), true);
  flush.interacting = () => false;
  assert.equal(registry.idle('scene', 'background'), false);
  flush.controlTarget = () => ({ sceneId: 'scene', backgroundId: 'background' });
  assert.equal(registry.idle('scene', 'background'), true);
  assert.equal(registry.idle('other', 'background'), false); assert.equal(registry.idle('scene', 'other'), false);
  for (const value of [undefined, null, 0, '']) {
    flush.interacting = () => value;
    assert.equal(registry.idle('scene', 'background'), false); assert.equal(registry.interacting(), true);
  }
  flush.interacting = () => { throw Error('reader unavailable'); };
  assert.equal(registry.idle('scene', 'background'), false); assert.equal(registry.interacting(), true);
  const successor = async () => {}; successor.interacting = () => false; successor.controlTarget = flush.controlTarget;
  const current = registry.register(successor); old(); assert.equal(registry.idle('scene', 'background'), true);
  current(); assert.equal(registry.idle('scene', 'background'), false);
});

test('the real shared editor reader distinguishes mounted idle mode from gestures, text, erasing and durable flights', () => {
  const f = shared();
  assert.equal(f.active(), false, 'host-set busy is not authoring activity and cannot self-block the barrier');
  for (const name of ['edit', 'draft', 'marquee', 'storedDraft', 'erasing', 'rasterPreview', 'selectionReading', 'finishing', 'imageZooming', 'objectDragging', 'pending', 'historyPending']) {
    f.values[name] = {}; assert.equal(f.active(), true, name); delete f.values[name]; assert.equal(f.active(), false, name);
  }
  f.values.groupDraft = [{}]; assert.equal(f.active(), true); delete f.values.groupDraft;
  for (const name of ['cancelGesture', 'flight', 'eraseFlight', 'tableCopy']) {
    f.scope[name] = {}; assert.equal(f.active(), true, name); f.scope[name] = undefined;
  }
  for (const name of ['strokeQueue', 'historyLane']) {
    f.scope[name].pending = () => true; assert.equal(f.active(), true, name); f.scope[name].pending = () => false;
  }
  f.scope.disposed = true; assert.equal(f.active(), true);
});

test('real object drag, resize, zoom, pending writes and focused text feed the same authoring reader', () => {
  const textarea = {}, document = { activeElement: undefined };
  const textActive = vm.runInNewContext(textReader, { textarea, document });
  const object = { disposeGesture: undefined, gestureFlight: undefined, zoomAsset: undefined, pinning: () => false, textFlush: Object.assign(async () => {}, { interacting: textActive }) };
  const activeObject = vm.runInNewContext(objectReader, object);
  const objectFlushes = new Map([['object', Object.assign(async () => {}, { interacting: activeObject })]]);
  const activeObjects = vm.runInNewContext(objectReaders, { objectFlushes });
  const f = shared(); f.scope.props.objectsFlush = Object.assign(async () => {}, { interacting: activeObjects });
  assert.equal(f.active(), false);
  for (const name of ['disposeGesture', 'gestureFlight', 'zoomAsset']) {
    object[name] = {}; assert.equal(f.active(), true, name); object[name] = undefined;
  }
  document.activeElement = textarea; assert.equal(f.active(), true, 'text object is being edited');
  document.activeElement = undefined; assert.equal(f.active(), false);
  objectFlushes.set('unknown', async () => {}); assert.equal(f.active(), true, 'unknown nested owner is not idle');
  objectFlushes.delete('unknown'); assert.equal(f.active(), false);
});

test('the ordinary object registry reads all current owners and fails closed without changing or flushing them', () => {
  const registry = new VideoFlushRegistry();
  assert.equal(registry.interacting(), false);
  let calls = 0, active = false;
  const flush = Object.assign(async () => { calls++; }, { interacting: () => active });
  const remove = registry.register(flush);
  assert.equal(registry.interacting(), false);
  active = true; assert.equal(registry.interacting(), true); active = false;
  const unknown = registry.register(async () => {}); assert.equal(registry.interacting(), true); unknown();
  const throwing = registry.register(Object.assign(async () => {}, { interacting: () => { throw Error('unknown'); } }));
  assert.equal(registry.interacting(), true); throwing();
  remove(); assert.equal(registry.interacting(), false); assert.equal(calls, 0);
});
