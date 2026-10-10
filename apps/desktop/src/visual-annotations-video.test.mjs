// Actual product helpers and App send body + synthetic state/IPC, no model/UI.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';
import { parse } from '@babel/parser';
const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
async function moduleAt(path) {
  const mod = new vm.SourceTextModule(stripTypeScriptTypes(await raw(path), { mode: 'transform' }));
  await mod.link(() => { throw Error('Unexpected runtime dependency'); }); await mod.evaluate(); return mod.namespace;
}
const helpers = await moduleAt('./visual-annotations-send.ts');
const submission = await moduleAt('./scene-submission.ts');
const app = await raw('./App.tsx'), ast = parse(app, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
let sendNode;
function visit(node) { if (!node || typeof node !== 'object') return; if (node.type === 'FunctionDeclaration' && node.id?.name === 'send') sendNode = node; for (const value of Object.values(node)) Array.isArray(value) ? value.forEach(visit) : visit(value); }
visit(ast); assert.ok(sendNode);
const sendBody = stripTypeScriptTypes(app.slice(sendNode.start, sendNode.end), { mode: 'transform' });
const grant = { pluginId: 'mewu.core.annotations', pluginRevision: 1, contributionId: 'annotate' };
const plugin = () => ({ state: 'enabled', revision: 3, manifest: { id: 'mewu.annotations', contributions: [{ id: 'answer', kind: 'agent.visual-annotations', engine: 'host.vector-v1' }] } });
const fixture = () => ({ id: 'scene', agentId: 'agent', connectionId: 'connection', background: { id: 'background', width: 640, height: 360 }, closed: false, frozen: false, regions: [{ id: 'region', x: 0, y: 0, width: 200, height: 100, drawingRevision: 1, drawings: [] }], items: [
  { id: 'video', asset: { id: 'video-source', kind: 'video', path: 'synthetic.mp4', width: 640, height: 360 }, videoEdit: { revision: 2, sourceDurationTicks: 60_000_000, range: { startTicks: 25_000_000, endTicks: 40_000_000 } }, videoAnnotations: { revision: 1, objects: [] } },
  { id: 'image', asset: { id: 'image-source', kind: 'image', path: 'synthetic.png' } },
  { id: 'text', asset: { id: 'text-source', kind: 'text', path: 'synthetic.txt' } },
], refs: [{ kind: 'item', id: 'text' }, { kind: 'item', id: 'video' }, { kind: 'region', id: 'region' }, { kind: 'item', id: 'image' }], draft: '说明视频中的错误并原位指出', messages: [] });
const turn = () => new Promise(resolve => setImmediate(resolve));
function deferred() { let resolve; const promise = new Promise(yes => resolve = yes); return { resolve, promise }; }
function actualApp() {
  const f = { scene: fixture(), plugin: plugin(), pending: false, exiting: false, calls: [], errors: {}, drafts: {}, operations: 0, afterSave: undefined, flushVideo: undefined };
  const snapshot = () => ({ scenes: [f.scene] });
  const scope = { ...helpers, ...submission, t: value => value, disposed: false, commands: Promise.resolve(), exitFailure: undefined, autosaveVersion: 0, debounce: undefined, clearTimeout,
    scene: () => f.scene, snapshot, busy: () => false, recording: () => false, scroll: () => false, sending: () => f.pending, exitPreparing: () => f.exiting,
    draft: () => f.scene.draft, refs: () => f.scene.refs, visualAnnotationGrant: () => helpers.annotationGrant([f.plugin]), pluginSnapshot: () => ({ plugins: [f.plugin] }),
    spaceOperations: { begin() { f.operations++; return () => f.operations--; } }, voice: { cancel: async () => f.calls.push('voice') },
    flushVideo: async () => { f.calls.push('video-flush'); await f.flushVideo?.(); }, flushDrawing: async () => f.calls.push('drawing-flush'), regionGeometry: { flush: async () => f.calls.push('geometry-flush') },
    setSending: value => f.pending = value, setSendErrors: update => f.errors = update(f.errors), setDrafts: update => f.drafts = update(f.drafts),
    accept: next => f.scene = next.scenes[0],
    bridge: {
      applyCommand: async command => { f.calls.push(structuredClone(command)); if (command.type === 'set_draft') f.scene.draft = command.draft; if (command.type === 'set_refs') f.scene.refs = structuredClone(command.refs); await f.afterSave?.(command); return snapshot(); },
      sendMessage: async (sceneId, annotation) => { f.calls.push({ type: 'send', sceneId, annotation: structuredClone(annotation) }); const next = structuredClone(f.scene); next.draft = ''; return { scenes: [next] }; },
    },
  };
  vm.createContext(scope); vm.runInContext(sendBody, scope); return { f, scope };
}

test('only one referenced real video exposes visual tools; no background required and selection never rewrites refs', () => {
  const scene = fixture(), before = JSON.stringify(scene.refs);
  assert.equal(helpers.hasAnnotationTarget(scene, scene.refs), true);
  delete scene.background; assert.equal(helpers.hasAnnotationTarget(scene, scene.refs), true);
  assert.equal(helpers.hasAnnotationTarget(scene, []), false);
  assert.equal(helpers.hasAnnotationTarget(scene, [{ kind: 'item', id: 'image' }]), false);
  scene.items.push({ id: 'video2', asset: { id: 'v2', kind: 'video' } });
  assert.equal(helpers.hasAnnotationTarget(scene, [...scene.refs, { kind: 'item', id: 'video2' }]), false);
  assert.equal(JSON.stringify(scene.refs), before);
});

test('source fence includes full video asset/range/document, all other attachments and ordered refs, while draft edits do not invalidate source', () => {
  const scene = fixture(), input = { scene, references: scene.refs, draft: scene.draft, grant, plugins: [plugin()], active: true, identity: helpers.annotationSendIdentity(scene), sourceIdentity: helpers.annotationSendIdentity(scene, scene.refs) };
  assert.doesNotThrow(() => helpers.assertAnnotationSend(input));
  for (const change of [s => s.items[0].asset.path = 'changed.mp4', s => s.items[0].videoEdit.range.startTicks++, s => s.items[0].videoAnnotations.objects.push({ sameRevision: true }), s => s.items[1].asset.path = 'changed.png', s => s.items[2].asset.path = 'changed.txt', s => s.regions[0].drawings.push({ id: 'new' })]) {
    const changed = structuredClone(scene); change(changed); assert.throws(() => helpers.assertAnnotationSend({ ...input, scene: changed }), /引用内容已变化/);
  }
  assert.throws(() => helpers.assertAnnotationSend({ ...input, references: [...scene.refs].reverse() }), /引用内容已变化/);
  assert.doesNotThrow(() => helpers.assertAnnotationSend({ ...input, scene: { ...scene, draft: '新草稿' } }));
  for (const forged of [{ ...grant, pluginId: 'mewu.annotations' }, { ...grant, pluginRevision: 2 }, { ...grant, contributionId: 'answer' }]) {
    assert.throws(() => helpers.assertAnnotationSend({ ...input, grant: forged }), /标注能力已变化/);
  }
  assert.doesNotThrow(() => helpers.assertAnnotationSend({ ...input, plugins: [] }));
  assert.doesNotThrow(() => helpers.assertAnnotationSend({ ...input, plugins: [{ ...plugin(), state: 'disabled' }] }));
});

test('actual App waits own trim flush before binding source, preserves all mixed refs and passes exact grant once', async () => {
  const { f, scope } = actualApp(), refs = JSON.stringify(f.scene.refs);
  f.flushVideo = async () => { f.scene.items[0].videoEdit.revision++; f.scene.items[0].videoEdit.range.startTicks = 30_000_000; };
  await scope.send();
  assert.equal(f.errors.scene, ''); assert.equal(f.operations, 0); assert.equal(f.pending, false);
  assert.deepEqual(f.calls.slice(0, 4), ['voice', 'video-flush', 'drawing-flush', 'geometry-flush']);
  const sent = f.calls.filter(c => c?.type === 'send'); assert.equal(sent.length, 1); assert.deepEqual(sent[0].annotation, grant);
  assert.equal(JSON.stringify(f.calls.find(c => c?.type === 'set_refs').refs), refs);
  assert.equal(f.scene.items[0].videoEdit.range.startTicks, 30_000_000);
});

test('actual App rejects source changes queued after flush, exit and failed flush without sending or losing draft', async () => {
  for (const mode of ['source', 'exit', 'flush']) {
    const { f, scope } = actualApp(), draft = f.scene.draft, refs = JSON.stringify(f.scene.refs), wait = deferred();
    if (mode === 'source' || mode === 'exit') scope.commands = wait.promise;
    if (mode === 'flush') f.flushVideo = async () => { throw Error('flush failed'); };
    const sending = scope.send(); await turn();
    if (mode === 'source') f.scene.items[0].videoAnnotations.objects.push({ changed: true });
    if (mode === 'exit') f.exiting = true;
    wait.resolve(); await sending;
    assert.equal(f.calls.some(c => c?.type === 'send'), false, mode); assert.ok(f.errors.scene, mode);
    assert.equal(f.scene.draft, draft); assert.equal(JSON.stringify(f.scene.refs), refs); assert.equal(f.operations, 0); assert.equal(f.pending, false);
  }
});

test('the single send exposes video tools without changing the user question or applying annotations itself', async () => {
  const { f, scope } = actualApp(), original = structuredClone(f.scene);
  await scope.send(); const sent = f.calls.find(c => c?.type === 'send');
  assert.deepEqual(sent.annotation, grant); assert.equal(f.errors.scene, '');
  assert.equal(f.calls.find(c => c?.type === 'set_draft').draft, original.draft);
  assert.equal(JSON.stringify(f.scene.items), JSON.stringify(original.items));
});

test('plain text and non-writable image attachments use the same send without a visual capability', async () => {
  for (const refs of [[], [{ kind: 'item', id: 'image' }], [{ kind: 'item', id: 'text' }]]) {
    const { f, scope } = actualApp(); f.scene.refs = refs;
    await scope.send(); const sent = f.calls.filter(c => c?.type === 'send');
    assert.equal(sent.length, 1); assert.equal(sent[0].annotation, undefined); assert.equal(f.errors.scene, '');
  }
});

test('a screenshot question or reference-only send exposes core optional tools even with a disabled legacy plugin', async () => {
  for (const mode of ['question', 'refs-only', 'disabled']) {
    const { f, scope } = actualApp(); f.scene.refs = [{ kind: 'region', id: 'region' }]; f.scene.draft = mode === 'refs-only' ? '' : '这是什么？';
    if (mode === 'disabled') f.plugin.state = 'disabled';
    const before = JSON.stringify(f.scene.regions);
    await scope.send(); const sent = f.calls.filter(c => c?.type === 'send');
    assert.equal(sent.length, 1); assert.equal(f.errors.scene, '');
    assert.deepEqual(sent[0].annotation, grant);
    assert.equal(JSON.stringify(f.scene.regions), before);
  }
});

test('capability target is bound after the local edit flush and legacy plugin changes cannot replace the core grant', async () => {
  for (const mode of ['refs-added', 'refs-removed', 'plugin-updated']) {
    const { f, scope } = actualApp();
    if (mode === 'refs-added') f.scene.refs = [];
    f.flushVideo = async () => { if (mode === 'refs-added') f.scene.refs = [{ kind: 'item', id: 'video' }]; if (mode === 'refs-removed') f.scene.refs = []; if (mode === 'plugin-updated') f.plugin.revision++; };
    await scope.send(); const sent = f.calls.filter(c => c?.type === 'send');
    assert.equal(sent.length, 1); assert.deepEqual(sent[0].annotation, mode === 'refs-removed' ? undefined : grant); assert.equal(f.errors.scene, '');
  }
});

test('disabled, removed or revised legacy plugins cannot revoke video core tools or alter the question', async () => {
  for (const mode of ['disabled', 'removed', 'revised-after-save']) {
    const { f, scope } = actualApp(), draft = f.scene.draft, refs = JSON.stringify(f.scene.refs), source = JSON.stringify(f.scene.items);
    if (mode === 'disabled') f.plugin.state = 'disabled';
    if (mode === 'removed') { scope.visualAnnotationGrant = () => helpers.annotationGrant([]); scope.pluginSnapshot = () => ({ plugins: [] }); }
    if (mode === 'revised-after-save') f.afterSave = async command => { if (command.type === 'set_refs') f.plugin.revision++; };
    await scope.send(); const sent = f.calls.filter(c => c?.type === 'send');
    assert.equal(sent.length, 1); assert.deepEqual(sent[0].annotation, grant); assert.equal(f.errors.scene, '');
    assert.equal(f.calls.find(c => c?.type === 'set_draft').draft, draft);
    assert.equal(JSON.stringify(f.scene.refs), refs); assert.equal(JSON.stringify(f.scene.items), source);
  }
});
