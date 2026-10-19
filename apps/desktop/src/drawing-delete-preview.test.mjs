// SPDX-License-Identifier: MPL-2.0
// Execute the actual browser-preview transaction adapter; no native IO.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';
import test from 'node:test';
const source = await readFile(new URL('./bridge.ts', import.meta.url), 'utf8');
const ast = parse(source, { sourceType: 'module', plugins: ['typescript'] });
const node = ast.program.body.find(node => node.type === 'ExportNamedDeclaration' && node.declaration?.id?.name === 'previewDrawingCommand').declaration;
const actual = stripTypeScriptTypes(source.slice(node.start, node.end), { mode: 'transform' });
const id = value => `00000000-0000-4000-8000-${String(value).padStart(12, '0')}`;
const ink = value => ({ id: id(value), kind: 'pen', color: '#ff0000', strokeWidth: 4, points: [{ x: 1, y: value }, { x: 20, y: value }] });
const plain = value => JSON.parse(JSON.stringify(value));
function fixture(drawings = [ink(1), ink(2), ink(3)]) {
  const region = { id: id(20), drawings, drawingRevision: 0, drawingHistory: { undo: [], redo: [] } };
  const scene = { id: id(21), background: { id: id(22), width: 100, height: 100 }, regions: [region], frozen: false, closed: false };
  const preview = { activeSceneId: scene.id, scenes: [scene] };
  const f = { preview, publications: 0 }, scope = { native: false, preview, structuredClone,
    findScene: value => { assert.equal(value, scene.id); return scene; }, clone: () => structuredClone(preview),
    publish: () => { f.publications++; return structuredClone(preview); } };
  vm.createContext(scope); vm.runInContext(actual, scope);
  f.region = () => scene.regions[0];
  f.command = action => scope.previewDrawingCommand({ sceneId: scene.id, backgroundId: scene.background.id, regionId: region.id, expectedRevision: f.region().drawingRevision, ...action });
  return f;
}
test('preview atomic deletion publishes once and one Undo/Redo retains the exact object order', async () => {
  const f = fixture(), original = structuredClone(f.region().drawings);
  await f.command({ type: 'remove_drawings', drawingIds: [id(3), id(1)] });
  assert.equal(f.publications, 1); assert.equal(f.region().drawingRevision, 1);
  assert.deepEqual(f.region().drawings, [original[1]]); assert.equal(f.region().drawingHistory.undo.length, 1);
  assert.deepEqual(plain(f.region().drawingHistory.undo[0].batch.map(edit => edit.index)), [2, 0]);
  await f.command({ type: 'undo_drawing' }); assert.deepEqual(f.region().drawings, original);
  await f.command({ type: 'redo_drawing' }); assert.deepEqual(f.region().drawings, [original[1]]);
});
test('preview rejects every invalid member before changing the source document or its history', async () => {
  const f = fixture(), before = structuredClone(f.preview);
  for (const drawingIds of [[], [id(1), id(1)], [id(1), id(99)], Array.from({ length: 257 }, (_, index) => id(100 + index))]) {
    await assert.rejects(f.command({ type: 'remove_drawings', drawingIds }));
    assert.deepEqual(f.preview, before); assert.equal(f.publications, 0);
  }
  await assert.rejects(f.command({ type: 'remove_drawings', drawingIds: [id(1), id(2)], expectedRevision: 9 }));
  assert.deepEqual(f.preview, before); assert.equal(f.publications, 0);
});
test('preview parent removal carries attached repair in the same step and refuses direct repair deletion', async () => {
  const parent = { ...ink(1), kind: 'rich', rich: { kind: 'extracted' } };
  const root = { ...ink(2), kind: 'rich', rich: { kind: 'repair' } };
  const linked = { ...ink(3), kind: 'rich', rich: { kind: 'repair', parentId: parent.id } };
  const ordinary = ink(4), f = fixture([root, parent, linked, ordinary]), before = structuredClone(f.preview);
  await assert.rejects(f.command({ type: 'remove_drawings', drawingIds: [ordinary.id, linked.id] })); assert.deepEqual(f.preview, before);
  await f.command({ type: 'remove_drawings', drawingIds: [parent.id, ordinary.id] });
  assert.deepEqual(f.region().drawings, [root]); assert.equal(f.region().drawingHistory.undo.length, 1);
  await f.command({ type: 'undo_drawing' }); assert.deepEqual(f.region().drawings, [root, parent, linked, ordinary]);
});
