// node --experimental-vm-modules apps/desktop/src/drawing-properties.test.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./components/drawing-properties.ts', import.meta.url), 'utf8'), { mode: 'transform' }));
await module.link(() => { throw new Error('Unexpected runtime import'); }); await module.evaluate();
const { propertyDraft, changeProperty, drawingPreview, draftChanged, drawingPropertyCommand, drawingDraftKey, DrawingDraftCache, settleDrawingEdit } = module.namespace;
const plain = value => JSON.parse(JSON.stringify(value)), checks = [];
const target = { sceneId: 'scene', backgroundId: 'background', regionId: 'region' };
const original = { id: 'object', kind: 'line', color: '#ff0000', strokeWidth: 4, points: [{ x: 6, y: 8 }, { x: 90, y: 12 }] };
const first = propertyDraft(original, 7);
original.points[0].x = 999;
assert.equal(first.base.points[0].x, 6); assert.equal(first.drawing.points[0].x, 6);
let colorDraft = first;
for (const color of ['#ff0033', '#cc00ff', '#0088ff']) colorDraft = changeProperty(colorDraft, { color });
const update = drawingPropertyCommand(colorDraft, target);
assert.deepEqual(plain(update), { ...target, expectedRevision: 7, type: 'update_drawing', drawing: { ...plain(first.base), color: '#0088ff' } });
assert.equal(drawingPropertyCommand(changeProperty(colorDraft, { color: '#ff0000' }), target), undefined);
assert.equal(first.drawing.color, '#ff0000');
checks.push('Picker previews preserve a detached baseline; one final command changes only color, same final value is no-op');

const widened = changeProperty(first, { strokeWidth: 8, text: 'ignored', fontSize: 42 });
assert.equal(widened.drawing.strokeWidth, 8); assert.equal(widened.drawing.fontSize, undefined); assert.equal(widened.drawing.text, undefined);
const mosaic = propertyDraft({ ...first.base, kind: 'mosaic', strokeWidth: 12 }, 3);
assert.equal(draftChanged(changeProperty(mosaic, { color: '#000000', strokeWidth: 2, fontSize: 36 })), false);
const numbered = propertyDraft({ ...first.base, kind: 'number', text: '42', fontSize: 28, points: [{ x: 4, y: 6 }] }, 3);
const numberUpdate = changeProperty(numbered, { color: '#1188ff', fontSize: 36, text: '43', strokeWidth: 18 });
assert.equal(numberUpdate.drawing.text, '42'); assert.equal(numberUpdate.drawing.strokeWidth, 4); assert.equal(numberUpdate.drawing.fontSize, 36);
checks.push('Controls only affect valid properties; sequence value, geometry, mosaic pixels and unused fields cannot be overwritten');
const provenance = { runId: 'run-a', userMessageId: 'message-a', toolEventId: 'event-a', groupId: 'batch-a', targetHandle: 'target-a', manifestSha256: 'a'.repeat(64) };
const annotated = { ...first.base, origin: { ...provenance } };
const annotatedDraft = changeProperty(propertyDraft(annotated, 9), { color: '#2255aa' });
annotated.origin.groupId = 'different';
const annotatedCommand = drawingPropertyCommand(annotatedDraft, target);
assert.deepEqual(plain(annotatedCommand.drawing.origin), provenance);
assert.equal(annotatedCommand.drawing.id, first.base.id);
assert.equal(annotatedCommand.expectedRevision, 9);
checks.push('Editing a model annotation preserves its immutable batch provenance and CAS without sharing the snapshot origin object');
const layers = [{ ...first.base, id: 'back' }, { ...first.base, id: 'middle' }, { ...first.base, id: 'front' }];
const preview = { ...layers[1], color: '#0088ff' };
assert.deepEqual(drawingPreview(layers, preview).map(value => value.id), ['back', 'middle', 'front']);
assert.equal(drawingPreview(layers, preview)[1], preview);
assert.equal(drawingPreview(layers, preview)[0], layers[0]); assert.equal(drawingPreview(layers, preview)[2], layers[2]);
assert.equal(drawingPreview(layers), layers);
assert.deepEqual(drawingPreview(layers, { ...preview, id: 'new' }).map(value => value.id), ['back', 'middle', 'front', 'new']);
checks.push('Existing preview replaces only its original layer slot; pointer selection and style edits never reorder the text DOM or raise annotations');

const textObject = { ...first.base, kind: 'text', points: [{ x: 20, y: 30 }], text: '原始文字', fontSize: 20 };
const textDraft = changeProperty(propertyDraft(textObject, 19, 'text'), { text: '  中文 <svg>\nsecond line  ', color: '#223344', fontSize: 28 });
const textCommand = drawingPropertyCommand(textDraft, target);
assert.equal(textCommand.type, 'update_drawing'); assert.equal(textCommand.drawing.id, textObject.id); assert.equal(textCommand.expectedRevision, 19);
assert.equal(textCommand.drawing.text, '  中文 <svg>\nsecond line  '); assert.deepEqual(plain(textCommand.drawing.points), textObject.points);
assert.throws(() => drawingPropertyCommand(changeProperty(textDraft, { text: '' }), target), /不能为空/);
assert.throws(() => drawingPropertyCommand(changeProperty(textDraft, { text: 'x'.repeat(2001) }), target), /2000/);
assert.throws(() => drawingPropertyCommand(changeProperty(textDraft, { text: 'a\0b' }), target), /控制字符/);
assert.equal(drawingPropertyCommand(propertyDraft({ ...textObject, text: '' }, 3, 'text', false), target), undefined);
assert.equal(drawingPropertyCommand(propertyDraft(textObject, 3, 'text', false), target).type, 'add_drawing');
checks.push('Existing text saves same ID/revision with literal whitespace and newlines; only fresh text adds and invalid text remains unsaved');

const cache = new DrawingDraftCache(2, 65536);
cache.set('A/bg/region/source', textDraft); cache.set('B/bg/region/source', colorDraft);
assert.equal(cache.get('A/bg/region/source'), textDraft);
const failed = { ...textDraft, error: 'CAS conflict' }; cache.set('A/bg/region/source', failed);
cache.delete('A/bg/region/source', textDraft); assert.equal(cache.get('A/bg/region/source'), failed);
assert.equal(failed.expectedRevision, 19);
assert.throws(() => cache.set('C/bg/region/source', textDraft), /未保存标注过多/);
assert.equal(cache.get('B/bg/region/source'), colorDraft);
cache.delete('A/bg/region/source', failed); assert.equal(cache.get('A/bg/region/source'), undefined);
const tiny = new DrawingDraftCache(2, 32); assert.throws(() => tiny.set('中文', textDraft), /未保存标注过多/);
const cleanCache = new DrawingDraftCache(1, 65536);
for (let index = 0; index < 40; index++) cleanCache.remember(String(index), propertyDraft(textObject, 1, 'text'));
cleanCache.remember('dirty', textDraft); assert.equal(cleanCache.get('dirty'), textDraft);
cleanCache.remember('dirty', propertyDraft(textObject, 1, 'text')); assert.equal(cleanCache.get('dirty'), undefined);
checks.push('Unmount recovery retains original CAS; stale completion cannot erase newer drafts and capacity never evicts unsaved input');
const crop = { ...target, sourceId: 'source', x: 10, y: 12, width: 300, height: 200 };
assert.notEqual(drawingDraftKey(crop), drawingDraftKey({ ...crop, x: 11 }));
assert.notEqual(drawingDraftKey(crop), drawingDraftKey({ ...crop, sourceId: 'override' }));
cache.set(drawingDraftKey(crop), textDraft);
assert.equal(cache.get(drawingDraftKey({ ...crop, x: 11 })), undefined);
assert.equal(cache.hasPending('scene'), true); assert.equal(cache.hasPending('other'), false); assert.equal(cache.hasPending(), true);
const listed = cache.list('scene')[0];
assert.equal(listed.key, drawingDraftKey(crop)); assert.equal(listed.draft, textDraft);
cache.set(listed.key, failed); assert.equal(cache.delete(listed.key, listed.draft), false); assert.equal(cache.get(listed.key), failed);
cache.set(listed.key, textDraft);

const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
let current = true; const slow = deferred();
const save = settleDrawingEdit(() => slow.promise, () => current, () => cache.delete(drawingDraftKey(crop), textDraft));
current = false; slow.resolve(); assert.equal(await save, false); assert.equal(cache.get(drawingDraftKey(crop)), undefined);
// Continuation cancellation is independent from success: only an exact persisted
// draft is deleted; a newer edit under the same target survives a late receipt.
cache.set(drawingDraftKey(crop), failed);
assert.equal(await settleDrawingEdit(async () => {}, () => false, () => cache.delete(drawingDraftKey(crop), textDraft)), false);
assert.equal(cache.get(drawingDraftKey(crop)), failed);
const failedSave = deferred(); const failure = settleDrawingEdit(() => failedSave.promise, () => true);
failedSave.reject(new Error('CAS conflict')); await assert.rejects(failure, /CAS/);
assert.equal(await settleDrawingEdit(async () => {}, () => true), true);
checks.push('A real late completion cannot continue after scene/source disposal; rejection is propagated rather than treated as saved');

// Sequential persisted whole-object history is safe because text writes now join
// the same explicit stack; unlike old WPF TextChanged, there are no hidden writes.
const savedColor = changeProperty(propertyDraft(textObject, 1), { color: '#00aaff' }).drawing;
const savedText = changeProperty(propertyDraft(savedColor, 2, 'text'), { text: '新版文本' }).drawing;
const undo = [{ before: textObject, after: savedColor }, { before: savedColor, after: savedText }];
let state = savedText;
for (const entry of [...undo].reverse()) { assert.deepEqual(plain(state), plain(entry.after)); state = entry.before; }
assert.deepEqual(plain(state), textObject);
for (const entry of undo) { assert.deepEqual(plain(state), plain(entry.before)); state = entry.after; }
assert.equal(state.text, '新版文本'); assert.equal(state.color, '#00aaff');
checks.push('Explicit text and style transactions undo and redo in sequence without an out-of-stack text mutation');
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
