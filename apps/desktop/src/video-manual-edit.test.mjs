// SPDX-License-Identifier: MPL-2.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';

const production = new URL('../../../../apps/desktop/src/', import.meta.url), urls = new Map();
async function source(name) { try { return await readFile(new URL(`./${name}.ts`, import.meta.url), 'utf8'); } catch { return readFile(new URL(`${name}.ts`, production), 'utf8'); } }
async function load(name) {
  if (urls.has(name)) return urls.get(name);
  let code = stripTypeScriptTypes(await source(name), { mode: 'transform' });
  for (const dependency of [...code.matchAll(/from ['"]\.\/([^'"]+)['"]/g)].map(value => value[1])) code = code.replaceAll(`'./${dependency}'`, JSON.stringify(await load(dependency))).replaceAll(`"./${dependency}"`, JSON.stringify(await load(dependency)));
  const url = `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`; urls.set(name, url); return url;
}
const manual = await import(await load('video-manual-edit'));
const { VideoManualEditor, VideoTextDraftCache, videoPoint, validateVideoTextRead } = manual;
const { VideoRequestLane, VideoFlushRegistry } = await import(await load('video-requests'));
const { videoAnnotationTarget, videoAnnotationIdentity } = await import(await load('video-annotations'));
const { TrimCanceled } = await import(await load('video-trim'));
const turn = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const origin = { runId: 'run', userMessageId: 'user', toolEventId: 'event', groupId: 'group', targetHandle: 'target', manifestSha256: 'a'.repeat(64), createdSha256: 'b'.repeat(64) };
function fixture(cache = new VideoTextDraftCache()) {
  let item = { id: 'video', x: .1, y: .2, width: .5, height: .4, asset: { id: 'asset', kind: 'video', path: 'synthetic.mp4', sha256: 'c'.repeat(64) }, videoEdit: { revision: 4, sourceDurationTicks: 100_000_000, range: { startTicks: 15_300_000, endTicks: 80_000_000 } }, videoAnnotations: { version: 1, clock: 'sourcePlaybackTicks', sourceId: 'asset', sourceDurationTicks: 100_000_000, sourceWidth: 1920, sourceHeight: 1080, revision: 7, undo: [], redo: [], objects: [
    { id: 'rect', interval: { startTicks: 15_300_000, endTicks: 27_800_000 }, origin: { ...origin }, primitive: { kind: 'rect', bounds: { x: 101.25, y: 80.5, width: 200.5, height: 90.25 }, style: { color: '#123456', strokeWidth: 2.5, opacity: .5 } } },
    { id: 'text', interval: { startTicks: 18_300_000, endTicks: 50_000_000 }, origin: { ...origin }, primitive: { kind: 'text', topLeft: { x: 400.25, y: 150.5 }, layout: { layoutId: 'old-layout', layoutSha256: 'd'.repeat(64), rasterSha256: 'e'.repeat(64), width: 180, height: 60 } } },
  ] } };
  const moves = [], reads = [], edits = []; let view, editable = true, pauses = 0;
  const content = { version: 1, text: '  原文\n中文\t标签  ', color: '#Ab12EF', fontSize: 24.5 };
  const moveReceipt = (action, base = item) => { const value = structuredClone(base); value.videoAnnotations.revision++; const object = value.videoAnnotations.objects.find(x => x.id === action.annotationId); if (object.primitive.kind === 'rect') Object.assign(object.primitive.bounds, action.toTopLeft); else object.primitive.topLeft = { ...action.toTopLeft }; value.videoAnnotations.undo.push({ id: 'one-step' }); return value; };
  const textReceipt = (base = item) => { const value = structuredClone(base); value.videoAnnotations.revision++; const object = value.videoAnnotations.objects.find(x => x.id === 'text'); object.primitive.layout = { layoutId: 'new-layout', layoutSha256: 'f'.repeat(64), rasterSha256: '1'.repeat(64), width: 240, height: 80 }; value.videoAnnotations.undo.push({ id: 'text-step' }); return value; };
  const lane = new VideoRequestLane(async id => { cancellations.push(id); }); const cancellations = [];
  let readWork = async request => ({ requestId: request.id, target: request.target, annotationId: request.annotationId, reference: request.reference, content: structuredClone(content) });
  let editWork = async () => { const result = textReceipt(); item = result; return result; };
  const editor = new VideoManualEditor({ authority: () => ({ key: videoAnnotationIdentity('scene', item), target: videoAnnotationTarget('scene', item), item, editable }), move: async (target, action) => { moves.push({ target, action }); const result = moveReceipt(action); item = result; return result; }, read: (target, annotationId, reference) => lane.request('text-read', id => { const request = { id, target, annotationId, reference }; reads.push(request); return readWork(request); }), edit: draft => lane.request('text-edit', id => { edits.push({ id, draft }); return editWork(draft, id); }), pause: () => { pauses++; }, publish: next => { view = next; }, cache });
  return { editor, cache, lane, moves, reads, edits, cancellations, content, moveReceipt, textReceipt, get item() { return item; }, set item(value) { item = value; }, get view() { return view; }, get pauses() { return pauses; }, set editable(value) { editable = value; }, set readWork(value) { readWork = value; }, set editWork(value) { editWork = value; } };
}

test('rect and text drag use source coordinates, fractional geometry and one immutable CAS/Undo', async () => {
  for (const id of ['rect', 'text']) {
    const f = fixture(), before = structuredClone(f.item), box = { left: 80, top: 120, width: 480, height: 270 }, dimensions = { width: 1920, height: 1080 };
    const start = videoPoint({ x: 130, y: 160 }, box, dimensions); assert.deepEqual(start, { x: 200, y: 160 });
    assert.equal(f.editor.begin(id, start), true);
    f.editor.move(videoPoint({ x: 140.5, y: 169.75 }, box, dimensions));
    const old = before.videoAnnotations.objects.find(x => x.id === id), from = old.primitive.kind === 'rect' ? old.primitive.bounds : old.primitive.topLeft;
    assert.deepEqual(f.view.move.bounds, { x: from.x + 42, y: from.y + 39, width: old.primitive.kind === 'rect' ? 200.5 : 180, height: old.primitive.kind === 'rect' ? 90.25 : 60 });
    await f.editor.completeMove(); await f.editor.completeMove();
    assert.equal(f.moves.length, 1); assert.deepEqual(f.moves[0].target, videoAnnotationTarget('scene', before)); assert.deepEqual(f.moves[0].action.fromTopLeft, { x: from.x, y: from.y });
    assert.equal(f.item.videoAnnotations.undo.length, 1); assert.equal(f.item.videoAnnotations.revision, 8);
    const result = f.item.videoAnnotations.objects.find(x => x.id === id); assert.deepEqual(result.origin, old.origin); assert.deepEqual(result.interval, old.interval);
    assert.deepEqual(f.item.videoEdit, before.videoEdit); assert.equal(f.pauses, 1);
  }
});

test('clamped no-op, cancel, blur/scope/doc changes and busy never write uncommitted motion', async () => {
  for (const cancel of [f => f.editor.cancelMove(), f => f.editor.interrupt(), f => { f.editable = false; f.editor.reconcile(); }, f => { f.item = { ...f.item, asset: { ...f.item.asset, sha256: '0'.repeat(64) } }; f.editor.reconcile(); }, f => { f.item.videoAnnotations.revision++; f.editor.reconcile(); }, f => f.editor.dispose()]) {
    const f = fixture(); f.editor.begin('rect', { x: 200, y: 160 }); f.editor.move({ x: 210, y: 170 }); cancel(f); await f.editor.completeMove(); assert.equal(f.moves.length, 0);
  }
  const f = fixture(); f.editor.begin('rect', { x: 0, y: 0 }); await f.editor.completeMove(); assert.equal(f.moves.length, 0);
  f.editor.begin('rect', { x: 0, y: 0 }); f.editor.move({ x: 50_000, y: -50_000 }); assert.deepEqual(f.view.move.bounds, { x: 1719.5, y: 0, width: 200.5, height: 90.25 }); f.editor.cancelMove();
  assert.throws(() => videoPoint({ x: 1, y: 1 }, { left: 0, top: 0, width: 0, height: 1 }, { width: 1, height: 1 }));
});

test('existing video hit layer moves vectors with integer origins and geometry bounds, preserving complete edge PNG references', async () => {
  const f = fixture(), reference = { layoutId: 'vector-layout', layoutSha256: 'a'.repeat(64), rasterSha256: 'b'.repeat(64), width: 180, height: 70, geometryBounds: { x: 8.25, y: 9.25, width: 160.5, height: 50.5 } };
  f.item.videoAnnotations.objects.push({ id: 'vector', interval: { startTicks: 0, endTicks: f.item.videoAnnotations.sourceDurationTicks }, primitive: { kind: 'vector', topLeft: { x: -8, y: -9 }, layout: reference } });
  assert.equal(f.editor.begin('vector', { x: 20, y: 20 }), true);
  f.editor.move({ x: 20.2, y: 20.2 }); await f.editor.completeMove(); assert.equal(f.moves.length, 0);
  f.editor.begin('vector', { x: 20, y: 20 }); f.editor.move({ x: 20.8, y: 20.8 });
  assert.deepEqual(f.view.move.bounds, { x: -7, y: -8, width: 180, height: 70 }); await f.editor.completeMove();
  assert.deepEqual(f.moves[0].action.toTopLeft, { x: -7, y: -8 }); assert.deepEqual(f.item.videoAnnotations.objects.at(-1).primitive.layout, reference);
  f.editor.begin('vector', { x: 20, y: 20 }); f.editor.move({ x: -5000, y: -5000 });
  assert.equal(f.view.move.bounds.x, -8); assert.equal(f.view.move.bounds.y, -9); f.editor.cancelMove();
  f.editor.begin('vector', { x: 20, y: 20 }); f.editor.move({ x: 5000, y: 5000 });
  assert.equal(f.view.move.bounds.x, Math.floor(1920 - 8.25 - 160.5)); assert.equal(f.view.move.bounds.y, Math.floor(1080 - 9.25 - 50.5)); f.editor.cancelMove();
  assert.equal(f.reads.length, 0);
});

test('old text is native exact content/ref; no-op edit skips rendering and malformed/stale reads reject', async () => {
  const f = fixture(); await f.editor.open('text'); assert.deepEqual(f.view.text.content, f.content); assert.equal(f.reads[0].reference.layoutId, 'old-layout'); await f.editor.save(); assert.equal(f.edits.length, 0);
  for (const alter of [raw => ({ ...raw, requestId: 'other' }), raw => ({ ...raw, reference: { ...raw.reference, width: 181 } }), raw => ({ ...raw, path: 'file.png' }), raw => ({ ...raw, content: { ...raw.content, text: 'x\0' } })]) {
    const raw = { requestId: 'request', target: f.reads[0].target, annotationId: 'text', reference: f.reads[0].reference, content: f.content }; assert.throws(() => validateVideoTextRead(alter(raw), 'request', raw.target, 'text', raw.reference));
  }
  const stale = fixture(), wait = deferred(); stale.readWork = () => wait.promise; const read = stale.editor.open('text'); await turn(); stale.item.videoAnnotations.revision++; wait.resolve({ requestId: stale.reads[0].id, target: stale.reads[0].target, annotationId: 'text', reference: stale.reads[0].reference, content: stale.content }); await assert.rejects(read, TrimCanceled); assert.equal(stale.view.text, undefined);
});

test('slow render is single flight; early own published snapshot does not revoke exact accepted receipt; flush waits', async () => {
  const f = fixture(), native = deferred(); await f.editor.open('text'); f.editor.input('新原文\n第二行'); const before = structuredClone(f.item); f.editWork = () => native.promise;
  const saved = f.editor.save(); await turn(); const flushed = f.editor.flush(() => true); let done = false; void flushed.then(() => { done = true; });
  f.editor.input('cannot overwrite pending'); await f.editor.open('rect'); assert.equal(f.edits.length, 1); assert.equal(f.view.text.content.text, '新原文\n第二行');
  const result = f.textReceipt(before); f.item = result; f.editor.reconcile(); assert.equal(f.cache.list().length, 1); assert.equal(done, false); assert.equal(f.view.text.error, undefined);
  native.resolve(result); await Promise.all([saved, flushed]); assert.equal(f.cache.list().length, 0); assert.equal(f.view.text, undefined); assert.equal(done, true); assert.equal(f.item.videoAnnotations.undo.length, 1);
  assert.equal(f.edits[0].draft.content.color, '#Ab12EF'); assert.equal(f.edits[0].draft.content.fontSize, 24.5); assert.deepEqual(f.item.videoAnnotations.objects[1].origin, before.videoAnnotations.objects[1].origin); assert.deepEqual(f.item.videoAnnotations.objects[1].interval, before.videoAnnotations.objects[1].interval);
});

test('failed/stale/oversized text remains recoverable across dispose; dirty capacity never evicts', async () => {
  const cache = new VideoTextDraftCache(), f = fixture(cache); await f.editor.open('text'); f.editor.input('保留草稿'); f.editWork = async () => { throw Error('渲染失败'); }; await assert.rejects(f.editor.save(), /渲染失败/); assert.equal(f.view.text.content.text, '保留草稿'); assert.equal(cache.list().length, 1); f.editor.dispose();
  const restored = fixture(cache); assert.equal(restored.view.text.content.text, '保留草稿'); restored.item.videoAnnotations.revision++; restored.editor.reconcile(); await assert.rejects(restored.editor.flush(() => true), /已变化/); assert.equal(cache.list().length, 1); await restored.editor.cancelText(); assert.equal(cache.list().length, 0);
  const invalid = fixture(); await invalid.editor.open('text'); invalid.editor.input('🙂'.repeat(501)); await assert.rejects(invalid.editor.save(), /无效/); assert.equal(invalid.edits.length, 0); assert.equal(invalid.view.text.content.text.length, 1002);
  const tiny = new VideoTextDraftCache(1), first = fixture(tiny); await first.editor.open('text'); first.editor.input('一'); const draft = first.view.text; assert.throws(() => tiny.remember({ ...draft, annotationId: 'another', content: { ...draft.content, text: '二' } }), /过多/); assert.equal(tiny.list()[0].content.text, '一');
});

test('late accepted receipt cannot erase a recovered newer draft, and cancellation cannot fake native commit failure', async () => {
  const cache = new VideoTextDraftCache(), old = fixture(cache), native = deferred(); await old.editor.open('text'); old.editor.input('older submitted'); old.editWork = () => native.promise; const saving = old.editor.save(); await turn(); old.editor.dispose();
  const newer = fixture(cache); newer.editor.input('newer recovered draft'); const result = old.textReceipt(); native.resolve(result); await saving; assert.equal(cache.list()[0].content.text, 'newer recovered draft'); assert.equal(newer.view.text.content.text, 'newer recovered draft');
  const f = fixture(), wait = deferred(); await f.editor.open('text'); f.editor.input('accepted after Esc'); f.editWork = () => wait.promise; const save = f.editor.save(); await turn(); const canceled = f.editor.cancelText(); await turn(); assert.equal(f.cancellations.length, 1); assert.equal(f.cache.list().length, 1); wait.resolve(f.textReceipt()); await Promise.all([save, canceled]); assert.equal(f.cache.list().length, 0); assert.equal(f.view.text, undefined);
  const reject = fixture(), stopped = deferred(); await reject.editor.open('text'); reject.editor.input('unaccepted'); reject.editWork = () => stopped.promise; const attempted = reject.editor.save(); const caught = attempted.catch(error => error); await turn(); const escape = reject.editor.cancelText(); await turn(); stopped.reject(new TrimCanceled()); await escape; assert.ok(await caught instanceof TrimCanceled); assert.equal(reject.cache.list().length, 0);
});

test('text request lane keeps physical receipt ownership and a flush registry waits pending native work', async () => {
  const cancelWait = deferred(), editWait = deferred(), cancellations = []; const lane = new VideoRequestLane(id => { cancellations.push(id); return cancelWait.promise; });
  const edit = lane.request('text-edit', () => editWait.promise); const later = lane.request('text-read', async () => 'unreachable'); await assert.rejects(later.promise, /进行中/); const cleanup = edit.cancel(); await turn(); let done = false; void cleanup.then(() => { done = true; }); editWait.resolve('accepted'); await turn(); assert.equal(done, false); cancelWait.resolve(); assert.equal(await edit.promise, 'accepted'); await cleanup; assert.equal(done, true);
  const f = fixture(), wait = deferred(), registry = new VideoFlushRegistry(); await f.editor.open('text'); f.editor.input('flush on exit'); f.editWork = () => wait.promise; registry.register(active => f.editor.flush(active)); const flush = registry.flush(() => true); await turn(); assert.equal(f.edits.length, 1); wait.resolve(f.textReceipt()); await flush; assert.equal(f.cache.list().length, 0);
});

function functionSource(file, name) {
  const ast = parse(file, { sourceType: 'module', plugins: ['typescript', 'jsx'] }), component = ast.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration;
  const node = component.body.body.find(node => node.type === 'FunctionDeclaration' && node.id.name === name); assert.ok(node, name); return stripTypeScriptTypes(file.slice(node.start, node.end), { mode: 'transform' });
}
test('actual component pointer capture handles one release, pointercancel/lost capture, activation and stable source geometry', async () => {
  const code = await readFile(new URL('./components/VideoArtifact.tsx', import.meta.url), 'utf8'), pointer = functionSource(code, 'annotationPointer');
  for (const end of ['pointerup', 'pointercancel', 'lostpointercapture']) {
    const f = fixture(); let capture = false, activated = 0; const listeners = new Map(); const node = { ownerSVGElement: { getBoundingClientRect: () => ({ left: 80, top: 120, width: 480, height: 270 }) }, addEventListener: (name, listener) => listeners.set(name, listener), removeEventListener: name => listeners.delete(name), setPointerCapture: () => { capture = true; }, hasPointerCapture: () => capture, releasePointerCapture: () => { capture = false; const lost = listeners.get('lostpointercapture'); lost?.({ pointerId: 1 }); } };
    const context = vm.createContext({ props: { item: f.item, busy: false, onActivate: () => { activated++; } }, manual: f.editor, videoPoint, pausePlayback() {}, setSelectedAnnotation() {}, video: { focus() {} }, stopAnnotationPointer: undefined, report: error => { throw error; } }); vm.runInContext(pointer, context);
    context.annotationPointer({ button: 0, pointerId: 1, clientX: 130, clientY: 160, currentTarget: node, preventDefault() {}, stopPropagation() {} }, 'rect');
    assert.equal(activated, 1); assert.equal(capture, true); listeners.get('pointermove')({ pointerId: 1, clientX: 140.5, clientY: 169.75, preventDefault() {}, stopPropagation() {} });
    listeners.get(end)({ pointerId: 1, clientX: 140.5, clientY: 169.75, preventDefault() {}, stopPropagation() {} }); await turn();
    assert.equal(f.moves.length, end === 'pointerup' ? 1 : 0); assert.equal(capture, false); assert.equal(listeners.size, 0);
  }
});

test('actual App text commit is queued/CAS fenced and PREPARING only admits owned flush; bad receipts never publish', async () => {
  const file = await readFile(new URL('./App.tsx', import.meta.url), 'utf8'), code = functionSource(file, 'applyVideoAnnotationText');
  for (const variant of ['normal', 'exit-blocked', 'exit-flush', 'stale-ref', 'bad-origin']) {
    const f = fixture(), calls = [], accepted = [], snapshot = { scenes: [{ id: 'scene', closed: false, frozen: false, items: [f.item] }] };
    const context = vm.createContext({ commands: Promise.resolve(), videoCommits: new Set(), exitFailure: undefined, videoFlushDepth: variant === 'exit-flush' ? 1 : 0, exitPreparing: () => variant.startsWith('exit'), recording: () => false, scroll: () => false, scene: () => snapshot.scenes[0], spaceOperations: { begin: () => () => {} }, videoAnnotationTarget, sameVideoAnnotationTarget: (a, b) => JSON.stringify(a) === JSON.stringify(b), videoAnnotationBridge: { editVideoAnnotationText: async (...args) => { calls.push(args); const result = f.textReceipt(); if (variant === 'bad-origin') result.videoAnnotations.objects[1].origin.runId = 'changed'; return { scenes: [{ id: 'scene', items: [result] }] }; } }, accept: value => accepted.push(value) }); vm.runInContext(code, context);
    const ref = structuredClone(f.item.videoAnnotations.objects[1].primitive.layout); if (variant === 'stale-ref') ref.width++;
    const result = context.applyVideoAnnotationText('request-id', videoAnnotationTarget('scene', f.item), 'text', ref, f.content);
    if (['exit-blocked', 'stale-ref', 'bad-origin'].includes(variant)) { await assert.rejects(result); assert.equal(accepted.length, 0); } else { await result; assert.equal(calls.length, 1); assert.equal(accepted.length, 1); assert.deepEqual(calls[0], ['request-id', videoAnnotationTarget('scene', f.item), 'text', ref, f.content]); }
    assert.equal(context.videoCommits.size, 0);
  }
});

test('busy freezes input without dropping drafts; owned flush saves, stale scope stays visible for copy/cancel', async () => {
  const f = fixture(); await f.editor.open('text'); f.editor.input('退出前草稿'); f.editable = false; f.editor.input('must not enter while inert'); assert.equal(f.view.text.content.text, '退出前草稿'); await assert.rejects(f.editor.save(), TrimCanceled); await f.editor.flush(() => true); assert.equal(f.edits.length, 1); assert.equal(f.cache.list().length, 0);
  const g = fixture(manual.videoTextDrafts); await g.editor.open('text'); g.editor.input('source changed draft'); g.editor.dispose(); assert.throws(() => manual.assertVideoTextDraftsSaved('scene'), /未保存/);
  const changed = structuredClone(g.item); changed.asset.id = 'replacement'; changed.videoAnnotations.sourceId = 'replacement'; let view;
  const recovered = new VideoManualEditor({ authority: () => ({ key: videoAnnotationIdentity('scene', changed), target: videoAnnotationTarget('scene', changed), item: changed, editable: true }), pause() {}, publish: value => { view = value; }, read() { throw Error('must not guess old text'); }, edit() { throw Error('must not rebase stale draft'); }, move() { throw Error('must not move'); } });
  assert.equal(view.text.content.text, 'source changed draft'); assert.equal(view.text.target.sourceId, 'asset'); assert.match(view.text.error, /已变化/); await assert.rejects(recovered.flush(() => true), /已变化/); await recovered.cancelText(); manual.assertVideoTextDraftsSaved(); recovered.dispose();
});

function walk(node, visit) { if (!node || typeof node !== 'object') return; visit(node); for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(item => walk(item, visit)); else if (value && typeof value === 'object') walk(value, visit); }
function handlerSource(file, selector, name) {
  const ast = parse(file, { sourceType: 'module', plugins: ['typescript', 'jsx'] }); let found;
  walk(ast, node => { if (node.type !== 'JSXOpeningElement' || !node.attributes.some(attr => attr.type === 'JSXAttribute' && attr.name.name === 'class' && attr.value?.value === selector)) return; found = node.attributes.find(attr => attr.type === 'JSXAttribute' && attr.name.type === 'JSXNamespacedName' && `${attr.name.namespace.name}:${attr.name.name.name}` === name)?.value?.expression; });
  assert.ok(found); return stripTypeScriptTypes(`const handler = ${file.slice(found.start, found.end)}; globalThis.handler = handler;`, { mode: 'transform' });
}
test('actual minimal editor handles Chinese IME/Shift Enter, Enter commit and Escape; outer capture does not steal Escape', async () => {
  const artifact = await readFile(new URL('./components/VideoArtifact.tsx', import.meta.url), 'utf8'), keyHandler = handlerSource(artifact, 'video-annotation-text-editor', 'on:keydown');
  for (const [key, isComposing, shiftKey, expected] of [['Enter', true, false, 'keep'], ['Enter', false, true, 'keep'], ['Enter', false, false, 'save'], ['Escape', true, false, 'keep'], ['Escape', false, false, 'cancel']]) {
    const f = fixture(), errors = []; await f.editor.open('text'); f.editor.input('中文\n新正文'); const context = vm.createContext({ manual: f.editor, report: error => errors.push(error) }); vm.runInContext(keyHandler, context); let stopped = 0, prevented = 0;
    context.handler({ key, isComposing, keyCode: isComposing ? 229 : 0, shiftKey, ctrlKey: false, altKey: false, metaKey: false, stopPropagation() { stopped++; }, preventDefault() { prevented++; } }); await turn();
    assert.equal(stopped, 1); assert.equal(prevented, expected === 'keep' ? 0 : 1); assert.equal(f.edits.length, expected === 'save' ? 1 : 0); assert.equal(Boolean(f.view.text), expected === 'keep'); assert.deepEqual(errors, []); if (expected === 'keep') await f.editor.cancelText();
  }
  const app = await readFile(new URL('./App.tsx', import.meta.url), 'utf8'), ast = parse(app, { sourceType: 'module', plugins: ['typescript', 'jsx'] }), body = ast.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration.body.body; let canceled = 0;
  for (const name of ['videoExportEscape', 'pinEscape']) {
    const declaration = body.find(node => node.type === 'VariableDeclaration' && node.declarations.some(value => value.id.name === name)); assert.ok(declaration);
    const context = vm.createContext({ exitPreparing: () => false, videoExport: () => true, voicePending: () => false, pendingPin: undefined, cancelPendingPin() {}, cancelPinOnEscape: () => { canceled++; }, cancelVideoExport: () => { canceled++; } });
    vm.runInContext(stripTypeScriptTypes(`${app.slice(declaration.start, declaration.end)}; globalThis.handler = ${name};`, { mode: 'transform' }), context);
    context.handler({ key: 'Escape', isComposing: false, keyCode: 27, target: { closest: selector => selector === '.video-annotation-text-editor' ? {} : null }, preventDefault() { throw Error('capture stole editor key'); }, stopImmediatePropagation() { throw Error('capture stole editor key'); } });
  }
  assert.equal(canceled, 0);
});

test('actual editor placement clamps measured resized rectangle while keeping original trim popup placement', async () => {
  const file = await readFile(new URL('./components/VideoArtifact.tsx', import.meta.url), 'utf8'), code = functionSource(file, 'place'), f = fixture(); await f.editor.open('text');
  let measured = { width: 300, height: 205 }, text, popup; const context = vm.createContext({ disposed: false, video: { getBoundingClientRect: () => ({ left: 360, top: 360, width: 480, height: 270 }), closest: () => null }, window: { innerWidth: 640, innerHeight: 480 }, document: { querySelector: () => null }, popover: undefined, textEditor: { getBoundingClientRect: () => measured }, manualView: () => f.view, props: { item: f.item }, placeVideoControls: () => ({ left: 50, top: 40, width: 300 }), setPlacement: update => { popup = update({ left: 50, top: 40, width: 300 }); }, setDrawingBox: update => { const value = update({ x: 0, y: 0, width: 0, height: 0 }); assert.deepEqual(structuredClone(value), { x: 360, y: 360, width: 480, height: 270 }); }, setTextPlacement: value => { text = value; } }); vm.runInContext(code, context);
  context.place(); assert.equal(text.left, 332); assert.equal(text.top, 267); assert.ok(text.top + measured.height <= 472); assert.deepEqual(popup, { left: 50, top: 40, width: 300 });
  measured = { width: 300, height: 143 }; context.place(); assert.equal(text.top, 329); assert.ok(text.top + measured.height <= 472);
  context.window.innerWidth = 300; context.window.innerHeight = 180; measured = { width: 284, height: 164 }; context.place(); assert.equal(text.left, 8); assert.equal(text.top, 8); assert.ok(text.left + measured.width <= 292); assert.ok(text.top + measured.height <= 172); await f.editor.cancelText();
});
