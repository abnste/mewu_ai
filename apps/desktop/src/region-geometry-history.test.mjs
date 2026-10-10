// node --experimental-vm-modules apps/desktop/src/region-geometry-history.test.mjs
// Real preview bridge/history model, synthetic state only. No native IPC or UI.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { webcrypto } from 'node:crypto';
import { loadProviderPresets, loadGeometryHistory, loadAssetImport } from './geometry-test-module.mjs';
const source = async file => stripTypeScriptTypes(await readFile(new URL(file, import.meta.url), 'utf8'), { mode: 'transform' });
const history = await loadGeometryHistory(), { geometryHistoryCommand, geometryHistoryReceipt, geometryHistoryKey } = history.namespace;
const importedAssets = await loadAssetImport();
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const policy = new SourceTextModule(await source('./connection-policy.ts')); await policy.link(() => { throw Error('Unexpected import'); });
const mosaic = new SourceTextModule(await source('./mosaic-preview.ts')); await mosaic.link(() => { throw Error('Unexpected import'); });
if (!globalThis.crypto) globalThis.crypto = webcrypto;
const bridge = new SourceTextModule(`${await source('./bridge.ts')}\nexport const seed = value => { previewGeometry.close(); preview = value; };`, { initializeImportMeta: meta => { meta.glob = () => ({}); } });
await bridge.link(async name => name === './provider-presets' ? await loadProviderPresets() : name === './asset-import' ? importedAssets : name === './region-geometry-history' ? history : name === './connection-policy' ? policy : name === './mosaic-preview' ? mosaic : name.endsWith('/core') ? synthetic({ isTauri: () => false, invoke: () => { throw Error('No native'); } }) : synthetic({ listen: () => { throw Error('No native'); } })); await bridge.evaluate();
const api = bridge.namespace, id = () => webcrypto.randomUUID(), checks = [];
const region = name => ({ id: name, x: 10, y: 20, width: 100, height: 80, drawingRevision: 0, drawings: [], drawingHistory: { undo: [], redo: [] } });
const fixture = () => {
  const scene = { id: id(), closed: false, frozen: false, background: { id: 'bg', width: 1000, height: 800 }, regions: [region('A'), region('B')], draft: 'keep draft', refs: [], messages: [] };
  const state = { schemaVersion: 1, activeSceneId: scene.id, scenes: [scene], agents: [], connections: [], memories: [], memoryStats: [], mcpServers: [], defaultConnectionId: null };
  api.seed(state);
  const edit = (name, x, editId) => { const r = scene.regions.find(v => v.id === name); return api.applyCommand({ type: 'set_region_geometry', sceneId: scene.id, regionId: name, backgroundId: 'bg', sourceId: r.imageOverride?.id ?? 'bg', expectedRevision: r.drawingRevision, from: { x: r.x, y: r.y, width: r.width, height: r.height }, to: { x, y: r.y, width: r.width, height: r.height }, ...(editId ? { editId } : {}) }); };
  const finish = editId => api.applyCommand({ type: 'finish_region_geometry_edit', sceneId: scene.id, editId });
  const replay = async direction => { const command = geometryHistoryCommand(scene, direction), entry = scene.geometryHistory[direction].at(-1); const result = await api.applyCommand(command); return geometryHistoryReceipt(result, command, direction === 'undo' ? entry.from : entry.to); };
  return { scene, state, edit, finish, replay };
};
{
  const f = fixture(); await f.edit('A', 11); await f.edit('B', 30); await f.edit('A', 15);
  assert.deepEqual(f.scene.geometryHistory.undo.map(v => v.regionId), ['A', 'B', 'A']);
  assert.equal((await f.replay('undo')).regionId, 'A'); assert.equal((await f.replay('undo')).regionId, 'B'); assert.equal((await f.replay('undo')).regionId, 'A');
  assert.deepEqual(f.scene.regions.map(r => r.x), [10, 10]); for (let i = 0; i < 3; i++) await f.replay('redo');
  assert.deepEqual(f.scene.regions.map(r => r.x), [15, 30]); assert.equal(f.scene.geometryHistory.revision, 9); assert.equal(f.scene.draft, 'keep draft'); assert.deepEqual(f.scene.messages, []);
  checks.push('A/B/A scene-wide order controls undo and redo independently of hovered region, with monotonically increasing history/region revisions');
}
{
  const f = fixture(), group = id(); await f.edit('A', 11, group); await f.edit('A', 12, group); await f.edit('A', 13, group);
  assert.equal(f.scene.geometryHistory.undo.length, 1); assert.equal(f.scene.geometryHistory.undo[0].from.x, 10); assert.equal(f.scene.geometryHistory.undo[0].to.x, 13); assert.equal(f.scene.geometryHistory.revision, 3);
  const before = JSON.stringify(f.state); await f.finish(id()); assert.equal(JSON.stringify(f.state), before); await f.finish(group); assert.equal(JSON.stringify(f.state), before);
  await f.edit('A', 14, group); assert.equal(f.scene.geometryHistory.undo.length, 2); await f.finish(group);
  checks.push('Same live editId merges persisted endpoints, exact Finish changes no snapshot bytes, and a reused sealed ID starts a new history entry');
}
{
  const f = fixture(), group = id(); await f.edit('A', 11, group); await f.edit('A', 11, id()); await f.edit('A', 12, group); assert.equal(f.scene.geometryHistory.undo.length, 2);
  await f.replay('undo'); const rev = f.scene.geometryHistory.revision; await f.edit('A', 11, id()); assert.equal(f.scene.geometryHistory.revision, rev); assert.equal(f.scene.geometryHistory.redo.length, 1);
  const reverse = id(); await f.edit('A', 15, reverse); await f.edit('A', 11, reverse); assert.equal(f.scene.geometryHistory.undo.length, 1); assert.equal(f.scene.geometryHistory.redo.length, 0);
  checks.push('No-op never writes history or clears redo but closes a mismatched transient group; net-zero merged movement disappears without resurrecting the old redo branch');
}
{
  const f = fixture(), group = id(); await f.edit('A', 12, group);
  f.scene.regions[0].drawings.push({ id: 'new-drawing' }); f.scene.regions[0].drawingRevision++;
  await f.edit('A', 15, group); assert.equal(f.scene.geometryHistory.undo.length, 2); await f.replay('undo'); assert.equal(f.scene.regions[0].drawings[0].id, 'new-drawing');
  f.scene.regions[0].ocr = { drawingRevision: f.scene.regions[0].drawingRevision, document: 'latest' }; f.scene.regions[0].translation = { drawingRevision: f.scene.regions[0].drawingRevision, overlay: 'latest' };
  await f.replay('undo'); assert.equal(f.scene.regions[0].ocr, undefined); assert.equal(f.scene.regions[0].translation, undefined);
  checks.push('Intervening drawing revisions prevent merging while geometry undo preserves the latest drawings and never resurrects ordinary-crop OCR/translation');
}
{
  const f = fixture(), r = f.scene.regions[0]; r.imageOverride = { id: 'long' }; await f.edit('A', 20); r.ocr = { drawingRevision: r.drawingRevision, document: 'new OCR' }; r.translation = { drawingRevision: r.drawingRevision, overlay: 'new translation' };
  await f.replay('undo'); assert.equal(r.ocr.document, 'new OCR'); assert.equal(r.translation.overlay, 'new translation'); assert.equal(r.ocr.drawingRevision, r.drawingRevision); await f.replay('redo'); assert.equal(r.translation.drawingRevision, r.drawingRevision); assert.equal(r.imageOverride.id, 'long');
  checks.push('Override replay retains current OCR/translation/assets and synchronizes their revisions instead of restoring stale content');
}
{
  const f = fixture(); await f.edit('A', 20); const command = geometryHistoryCommand(f.scene, 'undo'), before = JSON.stringify(f.state);
  for (const bad of [{ expectedHistoryRevision: 999 }, { expectedOperationId: id() }, { regionId: 'B' }, { sourceId: 'other' }, { backgroundId: 'other' }, { expectedRevision: 0 }, { from: { ...command.from, x: 19 } }]) { await assert.rejects(api.applyCommand({ ...command, ...bad })); assert.equal(JSON.stringify(f.state), before); }
  f.scene.run = { status: 'running' }; await assert.rejects(api.applyCommand(command)); assert.equal(f.scene.geometryHistory.undo.length, 1); delete f.scene.run;
  await f.replay('undo'); await assert.rejects(api.applyCommand(command));
  checks.push('Every history head/revision and region source/from fence rejects stale replay atomically; running agents are not canceled to perform undo');
}
{
  const f = fixture(); for (let i = 0; i < 55; i++) await f.edit('A', 11 + i); assert.equal(f.scene.geometryHistory.undo.length, 50);
  await f.replay('undo'); await api.applyCommand({ type: 'remove_region', sceneId: f.scene.id, regionId: 'A' }); assert.equal(f.scene.geometryHistory.undo.length, 0); assert.equal(f.scene.geometryHistory.redo.length, 0);
  const rev = f.scene.geometryHistory.revision; const snapshot = await api.getSnapshot(); assert.equal(snapshot.scenes[0].geometryHistory.revision, rev); assert.ok(rev > 0);
  checks.push('History capacity is 50 combined entries; removing a region prunes its history and redo while retaining an empty stack’s nonzero revision');
}
{
  const f = fixture(), name = '旧区域'.repeat(1500); f.scene.regions[0].id = name;
  for (let i = 0; i < 8; i++) await f.edit(name, 11 + i);
  const kept = f.scene.geometryHistory.undo.length; assert.ok(kept > 0 && kept < 8);
  assert.ok(new TextEncoder().encode(JSON.stringify(f.scene.geometryHistory)).byteLength < 64 * 1024);
  for (let i = 0; i < kept; i++) await f.replay('undo');
  for (let i = 0; i < kept; i++) await f.replay('redo');
  assert.equal(f.scene.regions[0].x, 18);
  const huge = fixture(), oversized = '界'.repeat(24_000); huge.scene.regions[0].id = oversized;
  const before = JSON.stringify(huge.state); await assert.rejects(huge.edit(oversized, 11), /数据过大/); assert.equal(JSON.stringify(huge.state), before);
  checks.push('Preview bounds UTF-8 history bytes with worst-case revision/number reserve, evicts oldest oversized-ID entries and rejects one over-budget edit atomically');
}
{
  const defaults = { key: 'z', ctrlKey: true, metaKey: false, shiftKey: false, altKey: false, repeat: false, isComposing: false };
  assert.equal(geometryHistoryKey(defaults), 'undo'); assert.equal(geometryHistoryKey({ ...defaults, shiftKey: true }), 'redo'); assert.equal(geometryHistoryKey({ ...defaults, key: 'Y' }), 'redo');
  for (const overrides of [{ repeat: true }, { isComposing: true }, { altKey: true }, { ctrlKey: false }, { key: 'y', shiftKey: true }]) assert.equal(geometryHistoryKey({ ...defaults, ...overrides }), undefined);
  checks.push('Undo/redo key mapping handles Ctrl/Cmd+Z, Shift+Z and Y without stealing IME, repeated accelerators or unrelated modifiers');
}
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
