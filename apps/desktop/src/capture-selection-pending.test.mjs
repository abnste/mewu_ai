// SPDX-License-Identifier: MPL-2.0
// Execute production pointer handlers and JSX with Solid's real reactive runtime.
// Only native persistence and DOM hit testing are synthetic; no desktop/user data.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const ts = source => stripTypeScriptTypes(source, { mode: 'transform' });
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const source = await raw('./components/SpaceCanvas.tsx'), nodes = [];
function walk(node) {
  if (!node || typeof node !== 'object') return;
  if (node.type) nodes.push(node);
  for (const [key, value] of Object.entries(node)) {
    if (key === 'loc') continue;
    if (Array.isArray(value)) value.forEach(walk); else if (value && typeof value === 'object') walk(value);
  }
}
walk(parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] }));
const slice = node => source.slice(node.start, node.end);
const variable = name => {
  const node = nodes.find(node => node.type === 'VariableDeclarator' && (node.id.name === name
    || node.id.type === 'ArrayPattern' && node.id.elements.some(value => value?.name === name)));
  assert.ok(node, `production declaration ${name}`); return `const ${slice(node)};`;
};
const handler = name => { const node = nodes.find(node => node.type === 'FunctionDeclaration' && node.id.name === name); assert.ok(node, name); return slice(node); };
const elementWithClass = name => nodes.find(node => node.type === 'JSXElement'
  && node.openingElement.attributes.some(attribute => attribute.name?.name === 'class' && attribute.value?.value === name));
const mask = elementWithClass('capture-mask'), outline = elementWithClass('capture-selection');
const selectionShow = nodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'Show' && node.start < outline.start && node.end > outline.end);
const toolbar = elementWithClass('capture-toolbar');
const toolbarShow = nodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'Show' && node.start < toolbar.start && node.end > toolbar.end);
const toolbarWhen = toolbarShow.openingElement.attributes.find(attribute => attribute.name?.name === 'when').value.expression;
const cleanup = nodes.find(node => node.type === 'CallExpression' && node.callee.name === 'onCleanup' && slice(node).includes('disposed = true'));
const sourceEffect = nodes.find(node => node.type === 'CallExpression' && node.callee.name === 'createEffect' && slice(node).includes('lastSource = source'));
assert.ok(mask && selectionShow && toolbarWhen && cleanup && sourceEffect);
const harness = `import { batch, createSignal, createMemo, createEffect, Show, For, untrack, onCleanup } from 'solid-js';
import { SnapSelectionGesture, snapResize } from './window-snap';
import { geometryTarget } from './region-geometry';
export default function PendingOwner(props) {
  let disposed = false, interactionVersion = 0, forceManualSelection, cancelGesture, cancelDeferredPointer, snapPointer, newSelectionSource, lastSource;
  const canvas = props.canvas;
  ${['selection', 'pendingSelection', 'active', 'activeItemId', 'activity', 'forceNew', 'toolbarVisible', 'imageExportOpen', 'pluginMenu', 'snapshotVersion', 'snapPreview'].map(variable).join('\n')}
  const backgroundError = () => false, drawingSession = () => undefined, imageDrawing = () => undefined;
  const exporting = () => false, settling = () => false, extensionRunning = () => false, extensionInteracting = () => false, ocrPending = () => undefined;
  const geometry = () => props.geometry, viewport = () => props.viewport, regionById = id => props.scene.regions.find(region => region.id === id);
  ${['activeRegion', 'display', 'regions', 'point', 'selectionPreview', 'blocked', 'snapUnavailable'].map(variable).join('\n')}
  const clamp = (value, low, high) => Math.max(low, Math.min(Math.max(low, high), value));
  const contains = (box, x, y, padding = 0) => x >= box.x - padding && x <= box.x + box.width + padding && y >= box.y - padding && y <= box.y + box.height + padding;
  const sameGeometry = (a, b) => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
  const candidateAt = () => props.automatic;
  const composerBox = () => { props.readLayout(); return undefined; };
  const toolbarBox = () => ({x:0,y:0,width:0,height:0}), TOLERANCE = 24;
  const clearHideTimer = () => {}, placeToolbar = () => props.placed.push(active()), endNudge = () => {};
  const setCodeFocused = () => {}, setViewport = () => {}, clearSnapPreview = () => setSnapPreview(undefined);
  const hideToolbar = () => setToolbarVisible(false), deferGeometryPointer = () => false;
  const ocrRequests = { cancel() {}, dispose() {} }, snapshotSelections = { bind: (...args) => props.bound.push(args) };
  const snapLoader = {dispose(){}}, snapMutations = {disconnect(){}}, toolbarObserver = undefined, flashTimer = undefined;
  const codeFocus = () => {}, leave = () => {}, focusReference = () => {}, recordKey = () => {}, releaseKey = () => {}, snapFocus = () => {}, hover = () => {};
  ${['setSelectionActivity', 'trackPointer', 'begin', 'editRegion', 'currentPendingSelection', 'clearPendingSelection', 'reconcilePendingSelection', 'revealToolbar', 'previewCandidateAt'].map(handler).join('\n')}
  ${['blur', 'resize', 'snapVisibility'].map(variable).join('\n')}
  createEffect(reconcilePendingSelection);
  ${slice(sourceEffect)};
  ${slice(cleanup)};
  Object.assign(props.control, { begin, editRegion, blur, resize, snapVisibility, previewCandidateAt, revealToolbar, cancel: () => cancelGesture?.(),
    state: () => ({selection:selection(),pending:pendingSelection(),active:active(),preview:selectionPreview(),toolbar:toolbarVisible()}),
    lateVersion: () => interactionVersion });
  return <div>${slice(mask)}${slice(selectionShow)}<Show when={${slice(toolbarWhen)}}><button aria-label="authorized-toolbar" /></Show></div>;
}`;
const compiled = ts(transformSync(harness, { filename: 'capture-selection-pending.tsx', babelrc: false, configFile: false,
  parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]] }).code);

async function fixture({ automatic, scale = 1, offset = { x: 0, y: 0 } } = {}) {
  const calls = [], completed = [], errors = [], placed = [], bound = [], activity = [], requests = [], geometryEnds = [], control = {};
  const listeners = new Map(), capture = new Set();
  const canvas = { addEventListener(name, fn) { (listeners.get(name) ?? (listeners.set(name, new Set()), listeners.get(name))).add(fn); },
    removeEventListener(name, fn) { listeners.get(name)?.delete(fn); }, setPointerCapture(id) { capture.add(id); },
    hasPointerCapture: id => capture.has(id), releasePointerCapture: id => capture.delete(id), focus() {} };
  class Element { closest(selector) { return selector === '.capture-surface' ? this : null; } }
  const target = new Element(); let layoutReads = 0, hitReads = 0, id = 0;
  const noEvents = { removeEventListener() {} }, document = { hidden: false, hasFocus: () => true,
    elementFromPoint() { hitReads++; return target; }, querySelector: () => undefined, ...noEvents };
  document.documentElement = noEvents;
  const window = { getSelection: () => ({ removeAllRanges() {} }), ...noEvents };
  const context = createContext({ console, queueMicrotask, setTimeout, clearTimeout, Element, Error, document, window,
    crypto: { randomUUID: () => `new-region-${++id}` }, innerWidth: 1200, innerHeight: 800, devicePixelRatio: 1 });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const core = synthetic(solid), universal = new SourceTextModule(await raw('../../../node_modules/solid-js/universal/dist/universal.js'), { context });
  await universal.link(() => core); await universal.evaluate();
  const renderer = universal.namespace.createRenderer({
    createElement: type => ({ type, children: [], props: {}, parent: null }), createTextNode: text => ({ text, children: [], parent: null }),
    replaceText: (node, text) => { node.text = text; }, setProperty: (node, key, value) => { node.props[key] = value; },
    insertNode: (parent, node, anchor) => { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); const index = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(index < 0 ? parent.children.length : index, 0, node); node.parent = parent; },
    removeNode: (parent, node) => { parent.children.splice(parent.children.indexOf(node), 1); node.parent = null; },
    isTextNode: node => 'text' in node, getParentNode: node => node.parent, getFirstChild: node => node.children[0], getNextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1],
  });
  const snap = new SourceTextModule(ts(await raw('./window-snap.ts')), { context }); await snap.link(() => { throw Error('unexpected snap import'); }); await snap.evaluate();
  const geometryModule = new SourceTextModule(ts(await raw('./region-geometry.ts')), { context }); await geometryModule.link(() => { throw Error('unexpected geometry import'); }); await geometryModule.evaluate();
  const owner = new SourceTextModule(compiled, { context });
  await owner.link(name => name === 'solid-js' ? core : name === 'test-renderer' ? synthetic(renderer) : name === './window-snap' ? snap : geometryModule); await owner.evaluate();
  const [scene, setScene] = solid.createSignal({ id: 'scene', background: { id: 'background', width: 1200, height: 800 }, regions: [], items: [] });
  const [inputLocked, setInputLocked] = solid.createSignal(false);
  const props = { get scene() { return scene(); }, get inputLocked() { return inputLocked(); }, canvas, control, automatic, geometry: { x: offset.x, y: offset.y, width: 1200, height: 800, scale },
    viewport: { width: 1200, height: 800, deviceScale: 1 }, busy: false, geometryView: {}, geometryController: { beginPointer: () => true, previewPointer() {}, endPointer: async committed => { geometryEnds.push(committed); } }, maskOpacity: .4, placed, bound, readLayout: () => { layoutReads++; },
    onAddRegion(region) { calls.push(region); const request = deferred(); requests.push(request); return request.promise; },
    onSelectionComplete: region => completed.push(region), onSelectionActivity: value => activity.push(value), onError: value => errors.push(value) };
  const [mounted, setMounted] = solid.createSignal(true), root = renderer.createElement('root');
  const stop = renderer.render(() => solid.createComponent(solid.Show, { get when() { return mounted(); }, get children() { return owner.namespace.default(props); } }), root);
  const dispose = () => { setMounted(false); stop(); };
  const all = () => { const nodes = []; const visit = node => { nodes.push(node); node.children.forEach(visit); }; visit(root); return nodes; };
  const outline = () => all().find(node => node.props?.class === 'capture-selection');
  const cutouts = () => all().filter(node => node.type === 'rect' && node.props.fill === 'black');
  const pointer = (x, y, extra = {}) => ({ pointerId: 1, button: 0, clientX: x, clientY: y, shiftKey: false, target, preventDefault() {}, stopPropagation() {}, ...extra });
  const emit = (name, event) => { for (const fn of [...listeners.get(name) ?? []]) fn(event); };
  const begin = (x = 100, y = 100) => control.begin(pointer(x, y));
  const drag = (x = 300, y = 260) => emit('pointermove', pointer(x, y));
  const up = (x = 300, y = 260) => emit('pointerup', pointer(x, y));
  const ack = (index = 0, patch = {}) => setScene(previous => ({ ...structuredClone(previous), regions: [...previous.regions, { ...calls[index], ...patch }] }));
  return { control, props, calls, requests, completed, errors, placed, bound, activity, geometryEnds, document, begin, drag, up, emit, pointer, ack, setScene, scene, setInputLocked, dispose, all, outline, cutouts,
    get layoutReads() { return layoutReads; }, get hitReads() { return hitReads; }, get pointerListeners() { return [...listeners.values()].reduce((n, set) => n + set.size, 0); } };
}

test('pointerup preserves the exact final selection/mask before any store acknowledgement, without authorizing tools', async () => {
  const f = await fixture({ scale: .5, offset: { x: 10, y: 20 } });
  try {
    f.begin(110, 120); f.drag(210, 220); const node = f.outline(); assert.ok(node);
    f.up(260, 270);
    assert.equal(f.outline(), node, 'the production preview stays mounted across pointerup');
    assert.deepEqual(JSON.parse(JSON.stringify(f.outline().props.style)), { left: '110px', top: '120px', width: '150px', height: '150px' });
    assert.equal(f.cutouts().length, 1); assert.equal(f.control.state().selection, undefined);
    assert.equal(f.scene().regions.length, 0); assert.equal(f.completed.length, 0);
    assert.equal(f.all().some(node => node.props?.['aria-label'] === 'authorized-toolbar'), false);
    assert.deepEqual(f.activity, [true, false]); assert.equal(f.pointerListeners, 0);
    await tick(); assert.equal(f.calls.length, 1);
    assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0])), { id: 'new-region-1', x: 200, y: 200, width: 300, height: 300 });
    assert.equal(f.outline(), node); f.ack();
    assert.equal(f.outline(), undefined); assert.equal(f.cutouts().length, 1, 'real saved mask replaces the temporary cutout');
    assert.equal(f.completed.length, 0); assert.equal(f.control.state().toolbar, false);
    f.requests[0].resolve(); await tick(); assert.equal(f.completed.length, 1); assert.equal(f.control.state().toolbar, true);
    assert.equal(f.completed[0].id, f.calls[0].id); assert.equal(f.control.state().pending, undefined);
  } finally { f.dispose(); }
});

test('resolving the native promise without a real matching snapshot cannot drop the outline or publish completion', async () => {
  const f = await fixture();
  try {
    f.begin(); f.drag(); f.up(); await tick(); const node = f.outline();
    f.requests[0].resolve(); await tick(); assert.equal(f.outline(), node); assert.equal(f.completed.length, 0); assert.equal(f.control.state().toolbar, false);
    f.setScene(previous => structuredClone(previous)); assert.equal(f.outline(), node);
    f.ack(); assert.equal(f.outline(), undefined); assert.equal(f.completed.length, 1); assert.equal(f.control.state().toolbar, true);
  } finally { f.dispose(); }
});

test('failed persistence clears only the temporary selection and reports the failure once', async () => {
  const f = await fixture();
  try {
    f.begin(); f.up(); await tick(); assert.ok(f.outline()); f.requests[0].reject(Error('disk write failed')); await tick();
    assert.equal(f.outline(), undefined); assert.equal(f.cutouts().length, 0); assert.equal(f.control.state().active, '');
    assert.deepEqual(f.errors, ['disk write failed']); assert.deepEqual(f.completed, []); assert.equal(f.scene().regions.length, 0);
  } finally { f.dispose(); }
});

test('a later gesture owns the preview and a late old acknowledgement never steals its selection', async () => {
  const f = await fixture();
  try {
    f.begin(); f.up(); await tick(); f.begin(500, 400); f.up(700, 600); await tick();
    const node = f.outline(), latest = f.calls[1]; assert.equal(f.control.state().active, latest.id);
    f.ack(0); f.requests[0].resolve(); await tick(); assert.equal(f.outline(), node); assert.equal(f.control.state().active, latest.id); assert.deepEqual(f.completed, []);
    f.ack(1); f.requests[1].resolve(); await tick(); assert.deepEqual(f.completed.map(region => region.id), [latest.id]); assert.equal(f.control.state().active, latest.id);
  } finally { f.dispose(); }
});

test('a first selection acknowledgement during the next live drag cannot cancel the new same-source gesture', async () => {
  const f = await fixture();
  try {
    f.begin(); f.up(); await tick(); f.begin(500, 400); f.drag(650, 550);
    const node = f.outline(); f.ack(0); f.requests[0].resolve(); await tick();
    assert.equal(f.outline(), node); assert.equal(f.activity.at(-1), true); assert.ok(f.pointerListeners);
    f.drag(680, 580); f.up(700, 600); await tick(); assert.equal(f.calls.length, 2);
    assert.deepEqual(JSON.parse(JSON.stringify(f.calls[1])), { id: 'new-region-2', x: 500, y: 400, width: 200, height: 200 });
    f.ack(1); f.requests[1].resolve(); await tick(); assert.deepEqual(f.completed.map(region => region.id), [f.calls[1].id]);
  } finally { f.dispose(); }
});

test('true pixel-source/viewport changes still cancel new selection, while existing-region edit keeps its strict source-list guard', async () => {
  for (const change of ['background', 'geometry', 'viewport', 'exit', 'frozen']) {
    const f = await fixture();
    try {
      f.begin(); f.up(); await tick(); f.begin(500, 400); f.drag(650, 550);
      if (change === 'background') f.setScene(previous => ({ ...previous, background: { ...previous.background, id: 'new-background' } }));
      else {
        if (change === 'geometry') f.props.geometry.x++;
        if (change === 'viewport') f.props.viewport.deviceScale = 1.25;
        if (change === 'exit') f.setInputLocked(true);
        if (change === 'frozen') f.setScene(previous => ({ ...previous, frozen: true }));
        f.ack(0);
      }
      assert.equal(f.outline(), undefined, change); assert.equal(f.pointerListeners, 0, change);
      f.up(700, 600); f.requests[0].resolve(); await tick(); assert.equal(f.calls.length, 1, change); assert.deepEqual(f.completed, [], change);
    } finally { f.dispose(); }
  }
  const f = await fixture();
  try {
    const saved = { id: 'saved', x: 100, y: 100, width: 200, height: 160 };
    f.setScene(previous => ({ ...previous, regions: [saved] }));
    f.control.editRegion(f.pointer(300, 150), saved, 'e'); f.drag(330, 150); assert.ok(f.pointerListeners);
    f.setScene(previous => ({ ...previous, regions: [...previous.regions, { id: 'another', x: 500, y: 400, width: 20, height: 20 }] }));
    assert.equal(f.pointerListeners, 0); assert.deepEqual(f.geometryEnds, [false]);
  } finally { f.dispose(); }
});

test('exact automatic selection is held across the same-ID cloned scene and binds only the real acknowledgement', async () => {
  const automatic = { id: 7, selectable: true, snapshot: true, children: [], bounds: { x: 80, y: 70, width: 400, height: 300 } };
  const f = await fixture({ automatic });
  try {
    f.begin(); f.up(100, 100); await tick(); assert.equal(f.calls[0].x, 80); assert.equal(f.calls[0].width, 400); assert.equal(f.bound.length, 0);
    const node = f.outline(); f.setScene(previous => structuredClone(previous)); assert.equal(f.outline(), node);
    f.ack(); assert.equal(f.bound.length, 0); f.requests[0].resolve(); await tick(); assert.equal(f.bound.length, 1); assert.equal(f.bound[0][0].id, f.calls[0].id); assert.equal(f.bound[0][1].id, automatic.id);
  } finally { f.dispose(); }
});

test('a mismatching saved geometry or replacement image cannot acknowledge the pending selection', async () => {
  for (const patch of [{ width: 201 }, { imageOverride: { id: 'other-image' } }]) {
    const f = await fixture();
    try {
      f.begin(); f.up(); await tick(); f.ack(0, patch); assert.equal(f.outline(), undefined); assert.equal(f.control.state().pending, undefined);
      f.requests[0].resolve(); await tick(); assert.deepEqual(f.completed, []); assert.equal(f.control.state().active, '');
    } finally { f.dispose(); }
  }
});

test('a saved region removed before completion never reappears as a pending rectangle', async () => {
  const f = await fixture();
  try {
    f.begin(); f.up(); await tick(); f.ack(); assert.equal(f.outline(), undefined);
    f.setScene(previous => ({ ...previous, regions: [] })); assert.equal(f.outline(), undefined); assert.equal(f.control.state().pending, undefined);
    f.requests[0].resolve(); await tick(); assert.deepEqual(f.completed, []); assert.equal(f.control.state().active, '');
  } finally { f.dispose(); }
});

test('explicitly selecting another real region or hiding before dispatch cancels only the pending presentation intent', async () => {
  const f = await fixture();
  try {
    f.begin(); f.up(); await tick();
    const saved = { id: 'already-saved', x: 5, y: 5, width: 20, height: 20 };
    f.setScene(previous => ({ ...previous, regions: [saved] })); f.control.revealToolbar(saved);
    assert.equal(f.outline(), undefined); assert.equal(f.control.state().active, saved.id);
    f.ack(); f.requests[0].resolve(); await tick(); assert.equal(f.control.state().active, saved.id); assert.deepEqual(f.completed, []);
  } finally { f.dispose(); }
  const canceled = await fixture();
  try {
    canceled.begin(); canceled.up(); canceled.document.hidden = true; canceled.control.snapVisibility(); await tick();
    assert.equal(canceled.calls.length, 0); assert.equal(canceled.outline(), undefined); assert.deepEqual(canceled.completed, []);
  } finally { canceled.dispose(); }
});

test('scene/background replacement, close/freeze/exit, blur, hide, resize and disposal reject late presentation work', async () => {
  for (const action of ['scene', 'background', 'closed', 'frozen', 'running', 'exit', 'blur', 'hidden', 'resize', 'dispose']) {
    const f = await fixture(); let disposed = false;
    try {
      f.begin(); f.up(); await tick(); assert.ok(f.outline(), action);
      if (action === 'scene') f.setScene(previous => ({ ...previous, id: 'new-scene' }));
      else if (action === 'background') f.setScene(previous => ({ ...previous, background: { id: 'new-background' } }));
      else if (['closed', 'frozen'].includes(action)) f.setScene(previous => ({ ...previous, [action]: true }));
      else if (action === 'running') f.setScene(previous => ({ ...previous, run: { status: 'running' } }));
      else if (action === 'exit') f.setInputLocked(true);
      else if (action === 'hidden') { f.document.hidden = true; f.control.snapVisibility(); }
      else if (action === 'dispose') { f.dispose(); disposed = true; }
      else f.control[action]();
      assert.equal(f.outline(), undefined, action); f.requests[0].resolve(); await tick();
      assert.deepEqual(f.completed, [], action); assert.deepEqual(f.bound, [], action);
      if (!disposed) { assert.equal(f.control.state().pending, undefined, action); assert.equal(f.control.state().toolbar, false, action); }
    } finally { if (!disposed) f.dispose(); }
  }
});

test('tiny and canceled gestures never submit; blocked automatic preview avoids DOM hit/layout reads during active drag', async () => {
  const f = await fixture();
  try {
    f.begin(); const hits = f.hitReads, layouts = f.layoutReads;
    f.control.previewCandidateAt({ x: 200, y: 200, shiftKey: false }); assert.equal(f.hitReads, hits); assert.equal(f.layoutReads, layouts);
    f.up(102, 102); await tick(); assert.equal(f.calls.length, 0); assert.equal(f.outline(), undefined);
    f.begin(); f.drag(); f.control.cancel(); await tick(); assert.equal(f.calls.length, 0); assert.equal(f.outline(), undefined); assert.equal(f.pointerListeners, 0);
  } finally { f.dispose(); }
});
