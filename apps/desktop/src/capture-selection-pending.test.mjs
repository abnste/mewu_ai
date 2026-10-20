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
const savedSection = elementWithClass('capture-region');
const savedOpening = slice(savedSection.openingElement).slice(0, -1) + ' />';
const selectionShow = nodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'Show' && node.start < outline.start && node.end > outline.end);
const queuedOutline = nodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'For' && source.slice(node.start,node.end).includes('data-pending-selection="queued"'));
assert.ok(queuedOutline);
const toolbar = elementWithClass('capture-toolbar');
const toolbarShow = nodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'Show' && node.start < toolbar.start && node.end > toolbar.end);
const toolbarWhen = toolbarShow.openingElement.attributes.find(attribute => attribute.name?.name === 'when').value.expression;
const cleanup = nodes.find(node => node.type === 'CallExpression' && node.callee.name === 'onCleanup' && slice(node).includes('disposed = true'));
const sourceEffect = nodes.find(node => node.type === 'CallExpression' && node.callee.name === 'createEffect' && slice(node).includes('lastSource = source'));
const snapEffect = nodes.find(node => node.type === 'CallExpression' && node.callee.name === 'createEffect' && slice(node).includes('snapIdentity = identity'));
const snapOutline = elementWithClass('capture-snap-preview');
const snapShow = nodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'Show' && node.start < snapOutline.start && node.end > snapOutline.end);
assert.ok(mask && selectionShow && toolbarWhen && cleanup && sourceEffect && snapEffect && snapShow);
const harness = `import { batch, createSignal, createMemo, createEffect, Show, For, untrack, onCleanup } from 'solid-js';
import { SnapMapLoader, SnapSelectionGesture, snapImagePoint, snapTarget, snapResize } from './window-snap';
import { DrawingInputPhase, drawingDiagnosticsEnabled, recordDrawingInput } from './drawing-diagnostics';
import { beginObjectDrag } from './extension-object-drag';
import { geometryTarget, overlayRegion } from './region-geometry';
export default function PendingOwner(props) {
  let disposed = false, interactionVersion = 0, pendingSelectionEpoch = 0, forceManualSelection, cancelGesture, cancelDeferredPointer, snapPointer, newSelectionSource, lastSource, snapIdentity = '';
  const canvas = props.canvas;
  ${['selection', 'pendingSelection', 'pendingSelectionOutlines', 'active', 'activeItemId', 'activity', 'forceNew', 'toolbarVisible', 'imageExportOpen', 'pluginMenu', 'snapshotVersion', 'snapPreview', 'snapMap'].map(variable).join('\n')}
  const backgroundError = () => false, drawingSession = () => undefined, imageDrawing = () => undefined, flashedRegion = () => '';
  const exporting = () => false, settling = () => false, extensionRunning = () => false, extensionInteracting = () => false, ocrPending = () => undefined;
  const geometry = () => props.geometry, viewport = () => props.viewport, regionById = id => props.scene.regions.find(region => region.id === id);
  ${['regionIds', 'activeRegion', 'display', 'regions', 'point', 'selectionPreview', 'queuedSelectionPreviews', 'blocked', 'snapUnavailable'].map(variable).join('\n')}
  const clamp = (value, low, high) => Math.max(low, Math.min(Math.max(low, high), value));
  const contains = (box, x, y, padding = 0) => x >= box.x - padding && x <= box.x + box.width + padding && y >= box.y - padding && y <= box.y + box.height + padding;
  const sameGeometry = (a, b) => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
  const composerBox = () => { props.readLayout(); return props.composerBox; };
  const toolbarBox = () => ({x:0,y:0,width:0,height:0}), TOLERANCE = 24;
  const clearHideTimer = () => {}, placeToolbar = () => props.placed.push(active()), endNudge = () => {}, scheduleHide = () => {};
  const setCodeFocused = () => {}, setViewport = () => {};
  const hideToolbar = () => setToolbarVisible(false), deferGeometryPointer = () => false;
  const ocrRequests = { cancel() {}, dispose() {} }, snapshotSelections = { bind: (...args) => props.bound.push(args) };
  const snapMutations = {disconnect(){}}, toolbarObserver = undefined, flashTimer = undefined;
  const getWindowSnapMap = request => props.getWindowSnapMap(request);
  ${variable('snapLoader')}
  const codeFocus = () => {}, leave = () => {}, focusReference = () => {}, recordKey = () => {}, releaseKey = () => {}, snapFocus = () => {};
  ${['hover', 'setSelectionActivity', 'trackPointer', 'begin', 'editRegion', 'currentPendingSource', 'currentPendingSelection', 'removePendingOutline', 'retirePendingSelection', 'clearPendingSelection', 'presentPendingSelection', 'activateItem', 'reconcilePendingSelection', 'revealToolbar', 'candidateAt', 'previewCandidateAt', 'updateSnapPreview', 'clearSnapPreview'].map(handler).join('\n')}
  ${['blur', 'resize', 'snapVisibility'].map(variable).join('\n')}
  createEffect(reconcilePendingSelection);
  ${slice(snapEffect)};
  ${slice(sourceEffect)};
  ${slice(cleanup)};
  Object.assign(props.control, { begin, editRegion, activateItem, hover, blur, resize, snapVisibility, previewCandidateAt, updateSnapPreview, revealToolbar, cancel: () => cancelGesture?.(),
    state: () => ({selection:selection(),pending:pendingSelection(),active:active(),preview:selectionPreview(),toolbar:toolbarVisible()}),
    lateVersion: () => interactionVersion });
  return <div>${slice(mask)}${slice(snapShow)}<For each={regionIds()}>{(id,index)=>{
    const region=createMemo(()=>regionById(id)!);const box=createMemo(()=>display(region()),undefined,{equals:sameGeometry});return ${savedOpening};
  }}</For>${slice(selectionShow)}${slice(queuedOutline)}<Show when={${slice(toolbarWhen)}}><button aria-label="authorized-toolbar" /></Show></div>;
}`;
const compiled = ts(transformSync(harness, { filename: 'capture-selection-pending.tsx', babelrc: false, configFile: false,
  parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]] }).code);

async function fixture({ automatic, map: suppliedMap, fetchMap, diagnostics = false, scale = 1, offset = { x: 0, y: 0 }, holdPresentation = false } = {}) {
  const calls = [], completed = [], errors = [], placed = [], bound = [], activity = [], requests = [], geometryEnds = [], control = {};
  const listeners = new Map(), capture = new Set();
  const canvas = { addEventListener(name, fn) { (listeners.get(name) ?? (listeners.set(name, new Set()), listeners.get(name))).add(fn); },
    removeEventListener(name, fn) { listeners.get(name)?.delete(fn); }, setPointerCapture(id) { capture.add(id); },
    hasPointerCapture: id => capture.has(id), releasePointerCapture: id => capture.delete(id), focus() {} };
  class Element { blocked = false; closest(selector) { return selector === '.capture-surface' ? this : this.blocked ? this : null; } }
  const target = new Element(); let layoutReads = 0, hitReads = 0, id = 0;
  const noEvents = { addEventListener() {}, removeEventListener() {} }, document = { hidden: false, hasFocus: () => true,
    elementFromPoint() { hitReads++; return target; }, querySelector: () => undefined, ...noEvents };
  document.documentElement = noEvents;
  const window = { getSelection: () => ({ removeAllRanges() {} }), ...noEvents };
  const presentationTasks=[];
  let diagnosticClocks = 0;
  const context = createContext({ console, TextEncoder, performance: { now: () => ++diagnosticClocks }, queueMicrotask:callback=>holdPresentation?presentationTasks.push(callback):queueMicrotask(callback), setTimeout, clearTimeout, Element, Error, document, window,
    crypto: { randomUUID: () => `new-region-${++id}` }, innerWidth: 1200, innerHeight: 800, devicePixelRatio: 1 });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const core = synthetic(solid), universal = new SourceTextModule(await raw('../../../node_modules/solid-js/universal/dist/universal.js'), { context });
  await universal.link(() => core); await universal.evaluate();
  const renderer = universal.namespace.createRenderer({
    createElement: type => ({ type, children: [], props: {}, parent: null, get isConnected(){return this.type==='root'||Boolean(this.parent?.isConnected);} }), createTextNode: text => ({ text, children: [], parent: null }),
    replaceText: (node, text) => { node.text = text; }, setProperty: (node, key, value) => { node.props[key] = value; },
    insertNode: (parent, node, anchor) => { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); const index = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(index < 0 ? parent.children.length : index, 0, node); node.parent = parent; },
    removeNode: (parent, node) => { parent.children.splice(parent.children.indexOf(node), 1); node.parent = null; },
    isTextNode: node => 'text' in node, getParentNode: node => node.parent, getFirstChild: node => node.children[0], getNextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1],
  });
  const snap = new SourceTextModule(ts(await raw('./window-snap.ts')), { context }); await snap.link(() => { throw Error('unexpected snap import'); }); await snap.evaluate();
  const diagnosticModule = new SourceTextModule(ts(await raw('./drawing-diagnostics.ts')), { context }); await diagnosticModule.link(() => { throw Error('unexpected diagnostics import'); }); await diagnosticModule.evaluate();
  const diagnosticRows = [];
  const stopDiagnostics = diagnostics ? diagnosticModule.namespace.startDrawingDiagnostics(true, async payload => diagnosticRows.push(...JSON.parse(payload).rows)) : () => {};
  const geometryModule = new SourceTextModule(ts(await raw('./region-geometry.ts')), { context }); await geometryModule.link(() => { throw Error('unexpected geometry import'); }); await geometryModule.evaluate();
  const toolboxModule = new SourceTextModule(ts(await raw('./extension-object-drag.ts')), { context }); await toolboxModule.link(() => { throw Error('unexpected toolbox import'); }); await toolboxModule.evaluate();
  const owner = new SourceTextModule(compiled, { context });
  await owner.link(name => name === 'solid-js' ? core : name === 'test-renderer' ? synthetic(renderer) : name === './window-snap' ? snap : name === './drawing-diagnostics' ? diagnosticModule : name === './extension-object-drag' ? toolboxModule : geometryModule); await owner.evaluate();
  const [scene, setScene] = solid.createSignal({ id: 'scene', background: { id: 'background', width: 1200, height: 800 }, regions: [], items: [] });
  const [inputLocked, setInputLocked] = solid.createSignal(false);
  const mapRequests = [];
  const props = { get scene() { return scene(); }, get inputLocked() { return inputLocked(); }, canvas, control, geometry: { x: offset.x, y: offset.y, width: 1200, height: 800, scale },
    getWindowSnapMap(request) { mapRequests.push(request); return fetchMap ? fetchMap(request) : Promise.resolve(suppliedMap ?? (automatic ? { version: 1, backgroundId: 'background', width: 1200, height: 800, roots: [automatic] } : null)); },
    viewport: { width: 1200, height: 800, deviceScale: 1 }, busy: false, geometryView: {}, geometryController: { beginPointer: () => true, previewPointer() {}, endPointer: async committed => { geometryEnds.push(committed); } }, maskOpacity: .4, placed, bound, readLayout: () => { layoutReads++; },
    onAddRegion(region) { calls.push(region); const request = deferred(); requests.push(request); return request.promise; },
    onSelectionComplete: region => completed.push(region), onSelectionActivity: value => activity.push(value), onError: value => errors.push(value) };
  const [mounted, setMounted] = solid.createSignal(true), root = renderer.createElement('root');
  const stop = renderer.render(() => solid.createComponent(solid.Show, { get when() { return mounted(); }, get children() { return owner.namespace.default(props); } }), root);
  await tick();
  const dispose = () => { setMounted(false); stop(); stopDiagnostics(); };
  const all = () => { const nodes = []; const visit = node => { nodes.push(node); node.children.forEach(visit); }; visit(root); return nodes; };
  const outline = () => all().find(node => node.props?.class === 'capture-selection');
  const cutouts = () => all().filter(node => node.type === 'rect' && node.props.fill === 'black');
  const pointer = (x, y, extra = {}) => ({ pointerId: 1, button: 0, clientX: x, clientY: y, shiftKey: false, target, preventDefault() {}, stopPropagation() {}, ...extra });
  const emit = (name, event) => { for (const fn of [...listeners.get(name) ?? []]) fn(event); };
  const begin = (x = 100, y = 100) => control.begin(pointer(x, y));
  const drag = (x = 300, y = 260) => emit('pointermove', pointer(x, y));
  const up = (x = 300, y = 260) => emit('pointerup', pointer(x, y));
  const ack = (index = 0, patch = {}) => setScene(previous => ({ ...structuredClone(previous), regions: [...previous.regions, { ...calls[index], ...patch }] }));
  return { control, props, calls, requests, completed, errors, placed, bound, activity, geometryEnds, document, target, mapRequests, diagnosticRows, begin, drag, up, emit, pointer, ack, setScene, scene, setInputLocked, dispose, all, outline, cutouts,
    flushPresentation:()=>{const tasks=presentationTasks.splice(0);tasks.forEach(callback=>callback());},
    get diagnosticClocks() { return diagnosticClocks; }, get layoutReads() { return layoutReads; }, get hitReads() { return hitReads; }, get pointerListeners() { return [...listeners.values()].reduce((n, set) => n + set.size, 0); } };
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
    assert.equal(f.outline(), node, 'a snapshot alone must not remove the visible pending outline'); await tick();
    assert.equal(f.outline(), undefined); assert.equal(f.cutouts().length, 1, 'connected saved section and mask replace the temporary cutout');
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
    f.ack(); assert.equal(f.outline(), node); await tick(); assert.equal(f.outline(), undefined); assert.equal(f.completed.length, 1); assert.equal(f.control.state().toolbar, true);
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
      assert.equal(f.pointerListeners, 0, change, 'source cancellation is synchronous, independent of the old saved section handoff');
      await tick(); assert.equal(f.outline(), undefined, change);
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

test('capture-issued map reaches the production hover border and stationary release at display scale without a highlighted hole', async () => {
  const child = { id: 9, selectable: true, children: [], bounds: { x: 100, y: 90, width: 300, height: 200 } };
  const root = { id: 7, selectable: true, snapshot: true, children: [child], bounds: { x: 80, y: 70, width: 900, height: 650 } };
  const f = await fixture({ automatic: root, scale: .5, offset: { x: 10, y: 20 } });
  try {
    f.props.geometry.width = 600; f.props.geometry.height = 400;
    assert.equal(f.mapRequests.length, 1);
    f.control.updateSnapPreview({ x: 110, y: 100, shiftKey: false });
    const border = f.all().find(node => node.props?.class === 'capture-snap-preview');
    assert.ok(border);
    assert.deepEqual(JSON.parse(JSON.stringify(border.props.style)), { left: '60px', top: '65px', width: '150px', height: '100px' });
    assert.equal(f.cutouts().length, 0, 'hover supplies a border, never a brighter whole-window cutout');
    f.begin(110, 100);
    assert.equal(f.all().some(node => node.props?.class === 'capture-snap-preview'), false);
    assert.equal(f.calls.length, 0);
    f.up(110, 100); await tick();
    assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0])), { id: 'new-region-1', ...child.bounds });
  } finally { f.dispose(); }
});

test('missing invalid stale or occluded real maps cannot authorize a snap and manual selection still works', async () => {
  const root = { id: 7, selectable: true, children: [], bounds: { x: 80, y: 70, width: 900, height: 650 } };
  const map = { version: 1, backgroundId: 'background', width: 1200, height: 800, roots: [root] };
  for (const value of [null, { ...map, backgroundId: 'old-background' }, { ...map, width: 1199 },
    { ...map, roots: [root, root] }, { ...map, roots: [{ ...root, id: 8, selectable: false }, root] }]) {
    const f = await fixture({ map: value });
    try {
      f.control.updateSnapPreview({ x: 100, y: 100, shiftKey: false });
      assert.equal(f.all().some(node => node.props?.class === 'capture-snap-preview'), false);
      f.begin(); f.up(100, 100); await tick(); assert.equal(f.calls.length, 0);
      f.begin(); f.up(300, 260); await tick();
      assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0])), { id: 'new-region-1', x: 100, y: 100, width: 200, height: 160 });
    } finally { f.dispose(); }
  }
});

test('real map candidates preserve focus modal control composer existing-region and geometry ownership fences', async () => {
  const automatic = { id: 7, selectable: true, children: [], bounds: { x: 80, y: 70, width: 900, height: 650 } };
  for (const reason of ['focus', 'control', 'modal', 'composer', 'region', 'geometry', 'shift', 'busy']) {
    const f = await fixture({ automatic });
    try {
      if (reason === 'focus') f.document.hasFocus = () => false;
      if (reason === 'control') f.target.blocked = true;
      if (reason === 'modal') f.document.querySelector = () => ({});
      if (reason === 'composer') f.props.composerBox = { x: 80, y: 70, width: 100, height: 100 };
      if (reason === 'region') f.setScene(previous => ({ ...previous, regions: [{ id: 'saved', x: 80, y: 70, width: 100, height: 100 }] }));
      if (reason === 'geometry') f.props.geometryView.pending = true;
      if (reason === 'busy') f.props.busy = true;
      f.control.updateSnapPreview({ x: 100, y: 100, shiftKey: reason === 'shift' });
      assert.equal(f.all().some(node => node.props?.class === 'capture-snap-preview'), false, reason);
      const observed = [];
      assert.equal(f.control.previewCandidateAt({ x: 100, y: 100, shiftKey: reason === 'shift' }, code => observed.push(code)), undefined, reason);
      assert.deepEqual(observed, [{ focus: 3, control: 5, modal: 6, composer: 8, region: 9, geometry: 1, shift: 2, busy: 1 }[reason]], reason);
    } finally { f.dispose(); }
  }
});

test('snap instrumentation stays entirely off by default and enabled real handlers send only bounded numeric decisions', async () => {
  const automatic = { id: 7, selectable: true, children: [], bounds: { x: 80, y: 70, width: 900, height: 650 } };
  const disabled = await fixture({ automatic });
  try {
    disabled.begin(); disabled.up(100, 100); await tick();
    assert.equal(disabled.diagnosticClocks, 0); assert.deepEqual(disabled.diagnosticRows, []);
    assert.equal(disabled.calls.length, 1);
  } finally { disabled.dispose(); }
  const enabled = await fixture({ automatic, diagnostics: true });
  try {
    enabled.document.hasFocus = () => false;
    enabled.begin(); enabled.up(100, 100); await tick(); assert.equal(enabled.calls.length, 0);
    enabled.document.hasFocus = () => true;
    enabled.begin(); enabled.up(100, 100); await tick(); assert.equal(enabled.calls.length, 1);
    await new Promise(resolve => setTimeout(resolve, 150));
    assert.deepEqual(enabled.diagnosticRows.filter(row => row.phase === 20 || row.phase === 21).map(row => [row.phase, row.code, row.count, row.focused]), [[20, 0, 1, true], [21, 3, 0, false], [21, 0, 0, true]]);
    assert.ok(enabled.diagnosticRows.every(row => Object.keys(row).sort().join(',') === 'active,atMicros,button,buttons,code,count,flags,focused,phase,pointer,sequence,target'));
  } finally { enabled.dispose(); }
});

test('region diagnostics distinguish delivered new selections, pending saves and actual native replies without authorizing an outline', async () => {
  const f = await fixture({ diagnostics: true });
  try {
    f.begin(); f.up(); await tick(); assert.equal(f.calls.length, 1);
    assert.equal(f.scene().regions.length, 0); assert.equal(f.completed.length, 0);
    f.requests[0].resolve(); await tick();
    await new Promise(resolve => setTimeout(resolve, 150));
    assert.deepEqual(f.diagnosticRows.filter(row => row.phase === 22 || row.phase === 23).map(row => [row.phase, row.code]), [[22, 0], [23, 0], [23, 4], [23, 5]]);
    assert.equal(f.outline()?.props.class, 'capture-selection'); assert.equal(f.completed.length, 0);
    assert.equal(f.control.state().toolbar, false, 'a diagnostic native-reply mark is not a snapshot or presentation acknowledgement');
  } finally { f.dispose(); }
});

test('region diagnostics preserve rejected guard order, cancel and native failure while default-off production remains silent', async () => {
  const f = await fixture({ diagnostics: true });
  try {
    f.control.begin(f.pointer(100, 100, { button: 2 }));
    f.props.busy = true; f.begin(); f.props.busy = false;
    f.begin(); f.begin(500, 400); f.control.cancel();
    f.begin(); f.up(); await tick(); f.requests[0].reject(Error('original native error')); await tick();
    await new Promise(resolve => setTimeout(resolve, 150));
    assert.deepEqual(f.diagnosticRows.filter(row => row.phase === 22 || row.phase === 23).map(row => [row.phase, row.code]), [[22, 1], [22, 3], [22, 0], [22, 2], [23, 1], [22, 0], [23, 0], [23, 4], [23, 6]]);
    assert.deepEqual(f.errors, ['original native error']); assert.equal(f.calls.length, 1);
    assert.equal(f.pointerListeners, 0); assert.equal(f.outline(), undefined); assert.deepEqual(f.completed, []);
  } finally { f.dispose(); }
});

test('region diagnostics separate existing-region edits from new-selection admission and retain real geometry completion', async () => {
  const f = await fixture({ diagnostics: true });
  try {
    const region = { id: 'saved', x: 80, y: 70, width: 100, height: 100 };
    f.setScene(value => ({ ...value, regions: [region] }));
    f.control.editRegion(f.pointer(100, 100), region); f.up(120, 120); await tick();
    f.control.editRegion(f.pointer(100, 100), region, 'se'); f.control.cancel(); await tick();
    await new Promise(resolve => setTimeout(resolve, 150));
    assert.deepEqual(f.diagnosticRows.filter(row => row.phase === 24).map(row => row.code), [0, 4, 0, 5]);
    assert.deepEqual(f.geometryEnds, [true, false]); assert.equal(f.calls.length, 0); assert.equal(f.pointerListeners, 0);
  } finally { f.dispose(); }
});

test('a late map reply only paints the current pixel source and cannot replace a down-time candidate', async () => {
  const requests = [], root = { id: 7, selectable: true, children: [], bounds: { x: 80, y: 70, width: 900, height: 650 } };
  const f = await fixture({ fetchMap: request => { const result = deferred(); requests.push({ request, ...result }); return result.promise; } });
  const mapFor = backgroundId => ({ version: 1, backgroundId, width: 1200, height: 800, roots: [root] });
  try {
    f.control.updateSnapPreview({ x: 100, y: 100, shiftKey: false });
    f.setScene(previous => ({ ...previous, background: { ...previous.background, id: 'successor' } }));
    requests[0].resolve(mapFor('background')); await tick();
    assert.equal(f.all().some(node => node.props?.class === 'capture-snap-preview'), false);
    assert.equal(requests.length, 2);
    f.begin();
    requests[1].resolve(mapFor('successor')); await tick();
    f.up(100, 100); await tick(); assert.equal(f.calls.length, 0, 'a click without a certified down candidate remains empty');
    f.control.updateSnapPreview({ x: 100, y: 100, shiftKey: false });
    assert.ok(f.all().find(node => node.props?.class === 'capture-snap-preview'));
    f.begin(); f.up(100, 100); await tick(); assert.equal(f.calls[0].width, 900);
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

test('automatic down does not flash a window-sized cutout and only an unmoved release confirms it', async () => {
  const automatic = { id: 7, selectable: true, snapshot: true, children: [], bounds: { x: 80, y: 70, width: 900, height: 650 } };
  const f = await fixture({ automatic });
  try {
    f.begin();
    assert.deepEqual(JSON.parse(JSON.stringify(f.control.state().selection)), { x: 100, y: 100, width: 0, height: 0 });
    assert.equal(f.cutouts().every(node => node.props.width === 0 && node.props.height === 0), true);
    await tick(); assert.equal(f.calls.length, 0); assert.equal(f.bound.length, 0);
    f.up(100, 100); await tick();
    assert.equal(f.calls.length, 1); assert.equal(f.calls[0].width, 900); assert.equal(f.calls[0].height, 650);
    assert.equal(f.bound.length, 0); assert.equal(f.control.state().toolbar, false);
    f.ack(); f.requests[0].resolve(); await tick(); assert.equal(f.bound.length, 1);
  } finally { f.dispose(); }
});

test('fast manual dragging within one snap candidate never exposes its full window bounds', async () => {
  const automatic = { id: 7, selectable: true, snapshot: true, children: [], bounds: { x: 80, y: 70, width: 900, height: 650 } };
  const f = await fixture({ automatic });
  try {
    f.begin(); f.drag(120, 125);
    assert.deepEqual(JSON.parse(JSON.stringify(f.control.state().selection)), { x: 100, y: 100, width: 20, height: 25 });
    assert.equal(f.cutouts().some(node => node.props.width === 900), false);
    f.up(300, 260); await tick();
    assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0])), { id: 'new-region-1', x: 100, y: 100, width: 200, height: 160 });
    f.ack(); f.requests[0].resolve(); await tick(); assert.equal(f.bound.length, 0);
  } finally { f.dispose(); }
});

test('dragging away then returning, or canceling a held automatic candidate, cannot turn into a snap click', async () => {
  const automatic = { id: 7, selectable: true, snapshot: true, children: [], bounds: { x: 80, y: 70, width: 900, height: 650 } };
  for (const mode of ['return', 'cancel', 'blur']) {
    const f = await fixture({ automatic });
    try {
      f.begin();
      if (mode === 'return') { f.drag(180, 170); f.up(100, 100); }
      else if (mode === 'cancel') f.control.cancel();
      else f.control.blur();
      await tick(); assert.equal(f.calls.length, 0, mode); assert.equal(f.bound.length, 0, mode);
      assert.equal(f.outline(), undefined, mode); assert.equal(f.pointerListeners, 0, mode);
    } finally { f.dispose(); }
  }
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
    f.begin(); f.up(); await tick(); const original=f.outline();f.ack(); assert.equal(f.outline(), original);
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

const pendingOutlines = f => f.all().filter(node => node.props?.class === 'capture-selection');
const outlineGeometry = f => pendingOutlines(f).map(node => JSON.parse(JSON.stringify(node.props.style))).sort((a,b) => parseInt(a.left)-parseInt(b.left));
async function addHeldSelections(f, count) {
  for (let index=0; index<count; index++) { f.begin(30+index*20,100); f.up(45+index*20,180); await tick(); }
}
test('several finished production selections keep every unacknowledged outline and cutout while tools remain unauthorized', async () => {
  const f=await fixture();try{
    await addHeldSelections(f,3);assert.equal(f.calls.length,3);assert.equal(f.scene().regions.length,0);
    assert.equal(pendingOutlines(f).length,3);assert.equal(f.cutouts().length,3);assert.deepEqual(outlineGeometry(f).map(box=>box.left),['30px','50px','70px']);
    assert.equal(f.control.state().active,f.calls[2].id);assert.equal(f.control.state().toolbar,false);assert.deepEqual(f.completed,[]);
    assert.equal(f.all().some(node=>node.props?.['aria-label']==='authorized-toolbar'),false);
    const oldest = pendingOutlines(f).find(node=>node.props.style.left==='30px'); assert.ok(oldest);
    f.begin(90,100); f.up(105,180); await tick(); assert.equal(pendingOutlines(f).length,4);
    assert.equal(pendingOutlines(f).find(node=>node.props.style.left==='30px'),oldest,'another completion must retain the older queued DOM node');
    f.requests[1].resolve(); await tick(); assert.equal(pendingOutlines(f).find(node=>node.props.style.left==='30px'),oldest,'an unrelated pending promise must not recreate queued DOM');
  }finally{f.dispose();}
  const skipped=await fixture();let admitted=0;try{skipped.props.onAddRegion=()=>{admitted++;throw Error('unadmitted callback ran');};skipped.begin();skipped.up();skipped.control.activateItem('successor-owner');await tick();assert.equal(admitted,0);assert.deepEqual(skipped.errors,[]);assert.equal(pendingOutlines(skipped).length,0);}finally{skipped.dispose();}
});
test('an older failed save removes only its own pending outline; late exact acknowledgements never steal the newest owner', async () => {
  const f=await fixture();try{
    await addHeldSelections(f,3);assert.equal(pendingOutlines(f).length,3);
    f.requests[0].reject(Error('older synthetic save failed'));await tick();assert.equal(pendingOutlines(f).length,2);assert.deepEqual(outlineGeometry(f).map(box=>box.left),['50px','70px']);
    assert.equal(f.control.state().active,f.calls[2].id);assert.deepEqual(f.completed,[]);assert.deepEqual(f.errors,['older synthetic save failed'],'an admitted older failure must remain visible without changing the newest owner');
    f.requests[0].reject(Error('repeated rejection'));await tick();assert.equal(f.errors.length,1);
    f.ack(1);f.requests[1].resolve();await tick();assert.equal(pendingOutlines(f).length,1);assert.equal(f.control.state().active,f.calls[2].id);assert.deepEqual(f.completed,[]);
    f.ack(2);f.requests[2].resolve();await tick();assert.equal(pendingOutlines(f).length,0);assert.deepEqual(f.completed.map(region=>region.id),[f.calls[2].id]);
  }finally{f.dispose();}
});
test('pending outline queue clears on true source exit and explicit region or media owner change without reviving from late replies', async () => {
  for(const action of ['scene','background','closed','frozen','running','exit','blur','hidden','resize','region-owner','media-owner','dispose']){
    const f=await fixture();let disposed=false;try{
      await addHeldSelections(f,2);assert.equal(pendingOutlines(f).length,2,action);
      if(action==='scene')f.setScene(value=>({...value,id:'successor'}));
      else if(action==='background')f.setScene(value=>({...value,background:{...value.background,id:'successor'}}));
      else if(['closed','frozen'].includes(action))f.setScene(value=>({...value,[action]:true}));
      else if(action==='running')f.setScene(value=>({...value,run:{status:'running'}}));
      else if(action==='exit')f.setInputLocked(true);
      else if(action==='hidden'){f.document.hidden=true;f.control.snapVisibility();}
      else if(action==='region-owner'){const saved={id:'saved-owner',x:800,y:500,width:50,height:50};f.setScene(value=>({...value,regions:[saved]}));f.control.revealToolbar(saved);}
      else if(action==='media-owner')f.control.activateItem('owned-media');
      else if(action==='dispose'){f.dispose();disposed=true;}
      else f.control[action]();
      assert.equal(pendingOutlines(f).length,0,action);f.requests[0].reject(Error('late failure after true clear'));for(const request of f.requests.slice(1))request.resolve();await tick();assert.equal(pendingOutlines(f).length,0,action);assert.deepEqual(f.completed,[],action);assert.deepEqual(f.errors,[],action+' must not inject a canceled presentation error');
    }finally{if(!disposed)f.dispose();}
  }
});
test('presentation queue is bounded to sixteen while every admitted selection still persists and the newest failed save leaves older outlines intact', async () => {
  const f=await fixture();try{
    await addHeldSelections(f,20);assert.equal(f.calls.length,20);assert.equal(pendingOutlines(f).length,16);assert.equal(f.scene().regions.length,0);assert.deepEqual(f.completed,[]);
    f.requests[0].reject(Error('evicted admitted failure'));await tick();assert.equal(pendingOutlines(f).length,16);assert.deepEqual(f.errors,['evicted admitted failure']);assert.equal(f.control.state().active,f.calls[19].id);
    f.requests[19].reject(Error('newest synthetic failure'));await tick();assert.equal(pendingOutlines(f).length,15);assert.deepEqual(f.errors,['evicted admitted failure','newest synthetic failure']);assert.equal(f.control.state().active,'');
    for(let i=1;i<19;i++){f.ack(i);f.requests[i].resolve();}await tick();assert.equal(f.scene().regions.length,18);assert.equal(pendingOutlines(f).length,0);assert.deepEqual(f.completed,[]);assert.equal(f.control.state().toolbar,false);
  }finally{f.dispose();}
});

const savedOutlines = f => f.all().filter(node => node.props?.class === 'capture-region');
test('a completed durable selection keeps its exact outline until the actual saved section is connected', async () => {
  const f=await fixture({holdPresentation:true});try{
    f.begin();f.drag();const original=f.outline();f.up();await tick();f.requests[0].resolve();await tick();
    assert.equal(f.outline(),original);assert.equal(savedOutlines(f).length,0);assert.equal(f.cutouts().length,1);assert.deepEqual(f.completed,[]);
    f.ack();const saved=savedOutlines(f)[0];assert.ok(saved?.isConnected);assert.deepEqual(JSON.parse(JSON.stringify(saved.props.style)),JSON.parse(JSON.stringify(original.props.style)));
    assert.equal(f.outline(),original,'receipt-before-attachment handoff cannot withdraw the only old outline');assert.equal(f.control.state().toolbar,false);assert.deepEqual(f.completed,[]);
    f.flushPresentation();assert.equal(f.outline(),undefined);assert.equal(savedOutlines(f)[0],saved);assert.equal(f.cutouts().length,1);
    assert.equal(f.control.state().toolbar,true);assert.deepEqual(f.completed.map(region=>region.id),[f.calls[0].id]);assert.deepEqual(f.errors,[]);
  }finally{f.dispose();}
});
test('saved section attachment before a delayed native reply replaces only presentation and grants no early toolbar', async () => {
  const f=await fixture({holdPresentation:true});try{
    f.begin();f.up();await tick();const original=f.outline();f.ack();const saved=savedOutlines(f)[0];assert.equal(f.outline(),original);
    f.flushPresentation();assert.equal(f.outline(),undefined);assert.equal(savedOutlines(f)[0],saved);assert.equal(f.control.state().toolbar,false);assert.deepEqual(f.completed,[]);
    f.setScene(value=>structuredClone(value));assert.equal(savedOutlines(f)[0],saved,'unrelated cloned snapshot preserves the already-connected section');
    f.requests[0].resolve();await tick();assert.equal(savedOutlines(f)[0],saved);assert.equal(f.control.state().toolbar,true);assert.equal(f.completed.length,1);
  }finally{f.dispose();}
});
test('multiple queued selections hand off their own connected outlines without stealing a newer pending selection', async () => {
  const f=await fixture({holdPresentation:true});try{
    await addHeldSelections(f,3);const newest=f.outline();assert.equal(pendingOutlines(f).length,3);
    f.ack(0);f.requests[0].resolve();await tick();assert.equal(pendingOutlines(f).length,3,'old pending border remains through its delayed section handoff');
    const oldestSaved=savedOutlines(f)[0];f.flushPresentation();assert.equal(pendingOutlines(f).length,2);assert.equal(savedOutlines(f)[0],oldestSaved);
    assert.equal(f.outline(),newest);assert.equal(f.control.state().active,f.calls[2].id);assert.deepEqual(f.completed,[]);
    f.ack(1);f.ack(2);f.requests[1].resolve();f.requests[2].resolve();await tick();assert.equal(pendingOutlines(f).length,2);assert.equal(f.control.state().toolbar,false);
    f.flushPresentation();assert.equal(pendingOutlines(f).length,0);assert.equal(savedOutlines(f).length,3);assert.equal(f.cutouts().length,3);
    assert.deepEqual(f.completed.map(region=>region.id),[f.calls[2].id]);assert.equal(f.control.state().active,f.calls[2].id);
  }finally{f.dispose();}
});

test('passive hover over an older saved region preserves a finished overlapping outline through delayed acknowledgement and DOM handoff', async () => {
  const f=await fixture({holdPresentation:true});try{
    const old={id:'existing-region',x:100,y:100,width:400,height:300};
    f.setScene(value=>({...value,regions:[old]}));
    f.begin(600,450);f.drag(400,300);f.up(400,300);await tick();
    const pending=f.outline();assert.ok(pending);assert.equal(f.calls.length,1);
    f.control.hover(f.pointer(400,301));
    assert.equal(f.outline(),pending,'ordinary movement after pointerup cannot withdraw the only unsaved border');
    assert.equal(f.control.state().active,f.calls[0].id);assert.equal(f.control.state().toolbar,false);
    assert.equal(f.scene().regions.length,1);assert.deepEqual(f.completed,[]);
    f.requests[0].resolve();await tick();assert.equal(f.outline(),pending);assert.deepEqual(f.completed,[]);
    f.ack();assert.equal(f.outline(),pending);assert.equal(savedOutlines(f).length,2);
    f.control.hover(f.pointer(400,301));assert.equal(f.outline(),pending);
    f.flushPresentation();assert.equal(f.outline(),undefined);assert.equal(savedOutlines(f).length,2);
    assert.deepEqual(f.completed.map(region=>region.id),[f.calls[0].id]);assert.deepEqual(f.errors,[]);
  }finally{f.dispose();}
});

test('passive hover cannot discard several admitted outlines while their independent saves and section attachments arrive', async () => {
  const f=await fixture({holdPresentation:true});try{
    const old={id:'existing-region',x:100,y:100,width:400,height:300};
    f.setScene(value=>({...value,regions:[old]}));
    for(let index=0;index<3;index++){
      f.begin(600+index*20,450+index*20);f.up(400+index*10,300+index*10);await tick();
      const pending=pendingOutlines(f);f.control.hover(f.pointer(400,301));
      assert.equal(pendingOutlines(f).length,index+1);assert.deepEqual(pendingOutlines(f),pending);
    }
    assert.equal(f.calls.length,3);assert.equal(f.scene().regions.length,1);assert.deepEqual(f.completed,[]);
    f.ack(0);f.requests[0].resolve();await tick();f.flushPresentation();
    assert.equal(pendingOutlines(f).length,2);f.control.hover(f.pointer(400,301));assert.equal(pendingOutlines(f).length,2);
    f.ack(2);f.requests[2].resolve();await tick();f.flushPresentation();
    assert.equal(pendingOutlines(f).length,1);assert.deepEqual(f.completed.map(region=>region.id),[f.calls[2].id]);
    f.ack(1);f.requests[1].resolve();await tick();f.flushPresentation();
    assert.equal(pendingOutlines(f).length,0);assert.equal(savedOutlines(f).length,4);assert.deepEqual(f.errors,[]);
  }finally{f.dispose();}
});

test('an explicit saved region press clears hovered pending presentation and its late reply cannot steal the action owner', async () => {
  const f=await fixture({holdPresentation:true});try{
    const old={id:'existing-region',x:100,y:100,width:400,height:300};
    f.setScene(value=>({...value,regions:[old]}));f.begin(600,450);f.up(400,300);await tick();
    f.control.hover(f.pointer(400,301));assert.equal(pendingOutlines(f).length,1);
    assert.equal(f.control.state().toolbar,false);assert.equal(f.all().some(node=>node.props?.['aria-label']==='authorized-toolbar'),false);
    f.control.editRegion(f.pointer(400,301),old);assert.equal(pendingOutlines(f).length,0);assert.equal(f.control.state().pending,undefined);
    f.control.cancel();await tick();
    f.ack();f.requests[0].resolve();await tick();f.flushPresentation();
    assert.equal(pendingOutlines(f).length,0);assert.equal(f.control.state().active,old.id);assert.deepEqual(f.completed,[]);
  }finally{f.dispose();}
});

test('a real background replacement clears hovered pending outlines and rejects their late attachment or error', async () => {
  const f=await fixture({holdPresentation:true});try{
    const old={id:'existing-region',x:100,y:100,width:400,height:300};
    f.setScene(value=>({...value,regions:[old]}));f.begin(600,450);f.up(400,300);await tick();
    f.control.hover(f.pointer(400,301));assert.equal(pendingOutlines(f).length,1);
    f.setScene(value=>({...value,background:{...value.background,id:'successor-background'},regions:[]}));
    assert.equal(pendingOutlines(f).length,0);assert.equal(f.control.state().pending,undefined);
    f.requests[0].reject(Error('late original-source save'));await tick();f.flushPresentation();
    assert.equal(pendingOutlines(f).length,0);assert.deepEqual(f.errors,[]);assert.deepEqual(f.completed,[]);
  }finally{f.dispose();}
});
test('a delayed saved-section handoff cannot restore cleared detached or successor-source presentation', async () => {
  for(const action of ['blur','resize','source','exit','detached']){
    const f=await fixture({holdPresentation:true});try{
      f.begin();f.up();await tick();f.ack();f.requests[0].resolve();await tick();assert.ok(f.outline());assert.deepEqual(f.completed,[]);
      if(action==='source')f.setScene(value=>({...value,background:{...value.background,id:'successor-background'},regions:[]}));
      else if(action==='exit')f.setInputLocked(true);
      else if(action==='detached')f.setScene(value=>({...value,regions:[]}));
      else f.control[action]();
      f.flushPresentation();assert.equal(f.outline(),undefined,action);assert.deepEqual(f.completed,[],action);assert.equal(f.control.state().toolbar,false,action);assert.deepEqual(f.errors,[],action);
    }finally{f.dispose();}
  }
});
