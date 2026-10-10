// node --experimental-vm-modules --test apps/desktop/src/drawing-resize.test.mjs
// Execute the real editor pointer handler with synthetic DOM events, never native IO.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
import test from 'node:test';

const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const transform = source => stripTypeScriptTypes(source, { mode: 'transform' });
const plain = value => JSON.parse(JSON.stringify(value));
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
async function pure(path) { const module = new SourceTextModule(transform(await raw(path))); await module.link(() => { throw Error('Unexpected runtime dependency'); }); await module.evaluate(); return module.namespace; }
const geometry = await pure('./components/drawing-geometry.ts'), properties = await pure('./components/drawing-properties.ts'), preview = await pure('./drawing-layout-preview.ts');
const imageCode=await raw('./components/DrawingEditor.tsx'),imageAst=parse(imageCode,{sourceType:'module',plugins:['typescript','jsx']});
const imageBody=imageAst.program.body.find(node=>node.type==='ExportDefaultDeclaration').declaration.body.body;
const imagePortCode=imageBody.filter(node=>node.type!=='ReturnStatement').map(node=>{let value=imageCode.slice(node.start,node.end);if(node.type==='VariableDeclaration'&&node.declarations[0].id.name==='port'){const render=node.declarations[0].init.properties.find(property=>property.key.name==='render').value;value=value.slice(0,render.start-node.start)+'()=>undefined'+value.slice(render.end-node.start);}return value;}).join('\n');
const imageMod=new SourceTextModule(transform(`import { drawingDraftKey,drawingPropertyCommand,richPreviewTarget,richSourceIdentity } from 'deps';const mosaicBlockSize=()=>8,getMosaicPreview=async()=>{},usesDrawingDocument=()=>false,richDocumentHistoryStep=()=>true;export function imagePort(props){${imagePortCode};return port;}`));await imageMod.link(()=>synthetic({...properties,...preview}));await imageMod.evaluate();
const code = await raw('./components/SharedDrawingEditor.tsx'), ast = parse(code, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const body = ast.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration.body.body;
function declaration(name) {
  const node = body.find(node => node.type === 'FunctionDeclaration' ? node.id.name === name : node.type === 'VariableDeclaration' && node.declarations.some(value => value.id.name === name));
  assert.ok(node, `Missing product declaration ${name}`); return code.slice(node.start, node.end);
}
const actual = ['cacheKey', 'gestureIdentity', 'point', 'target', 'handleSize', 'commit', 'beginResize', 'blur', 'resize'].map(declaration).join('\n');
const module = new SourceTextModule(transform(`
import { drawingResizeHandles, resizedDrawingPoints, drawingDraftKey, settleDrawingEdit, richSourceIdentity, imagePort } from 'dependencies';
export function fixture(props, svg, window) { props.port=imagePort(props);
  let cancelGesture, refreshConstraint, disposed = false, serial = 0, flight, pendingValue = false, draft, finishing = false;
  let selectedId = props.region.drawings[0].id, activeTool = 'select', editValue;
  const errors = [], tool = () => activeTool, selected = () => selectedId, edit = () => editValue;
  const selectedDrawing = () => props.region.drawings.find(value => value.id === selectedId);
  const pending = () => pendingValue, disabled = () => pending() || props.busy || finishing, setPending = value => pendingValue = value, setDraft = value => draft = value;
  const clamp = (value, low, high) => Math.max(low, Math.min(high, value));
  const report = error => errors.push(error.message), identity = () => cacheKey(), usesDrawingDocument = () => false;
  const surfaceSize = () => svg.getBoundingClientRect(), setViewport = () => {}, innerWidth = 1200, innerHeight = 800;
  ${actual}
  return { beginResize, draft: () => draft, pending, errors, flush: async () => { cancelGesture?.(); if (flight) return flight; return true; },
    cancel: () => cancelGesture?.(), blur, resize, handleSize, shift: value => refreshConstraint?.(value), tool: value => activeTool = value, edit: value => editValue = value,
    dispose: () => { disposed = true; cancelGesture?.(); }, hasGesture: () => !!cancelGesture };
}
`));
await module.link(() => synthetic({ drawingResizeHandles: geometry.drawingResizeHandles, resizedDrawingPoints: geometry.resizedDrawingPoints, drawingDraftKey: properties.drawingDraftKey, settleDrawingEdit: properties.settleDrawingEdit, richSourceIdentity: preview.richSourceIdentity, imagePort: imageMod.namespace.imagePort }));
await module.evaluate();
const region = { x: 100, y: 200, width: 400, height: 300 };
const shape = (kind = 'rect', points = [{ x: 140, y: 240 }, { x: 260, y: 320 }]) => ({ id: 'shape', kind, points, color: '#123456', strokeWidth: 7, origin: { runId: 'run', userMessageId: 'message', toolEventId: 'tool', groupId: 'group', targetHandle: 'target', manifestSha256: 'a'.repeat(64) } });
class Events {
  listeners = new Map();
  addEventListener(type, callback) { if (!this.listeners.has(type)) this.listeners.set(type, new Set()); this.listeners.get(type).add(callback); }
  removeEventListener(type, callback) { this.listeners.get(type)?.delete(callback); }
  dispatch(event) { for (const callback of [...(this.listeners.get(event.type) ?? [])]) callback(event); }
  count() { return [...this.listeners.values()].reduce((sum, entries) => sum + entries.size, 0); }
}
function setup(drawing = shape(), receive) {
  const window = new Events(), svg = new Events(), commands = [];
  svg.box = { left: 30, top: 40, width: 200, height: 75 }; // x=.5, y=.25 CSS/source, independent axes
  svg.getBoundingClientRect = () => ({ ...svg.box }); svg.focus = () => {};
  svg.setPointerCapture = id => { svg.captured = id; };
  svg.hasPointerCapture = id => svg.captured === id;
  svg.releasePointerCapture = id => { svg.captured = undefined; svg.dispatch(event('lostpointercapture', 0, 0, { pointerId: id })); };
  const props = { sceneId: 'scene', backgroundId: 'background', background: { id: 'background', path: 'assets/background.png', name: 'synthetic', kind: 'image', width: 600, height: 600 }, backgroundWidth: 600, backgroundHeight: 600, region: { ...region, id: 'region', drawingRevision: 17, drawings: [drawing] }, box: { x: 30, y: 40, width: 200, height: 75 }, busy: false,
    onCommand: command => { commands.push(plain(command)); return receive ? receive(command) : Promise.resolve(); } };
  return { ...module.namespace.fixture(props, svg, window), props, svg, window, commands };
}
function event(type, sourceX, sourceY, extra = {}) {
  return { type, button: 0, buttons: type === 'pointerup' ? 0 : 1, pointerId: 3, clientX: 30 + (sourceX - region.x) * .5, clientY: 40 + (sourceY - region.y) * .25, shiftKey: false, preventDefault() {}, stopPropagation() {}, ...extra };
}
const fire = (fixture, type, x, y, extra) => fixture.window.dispatch(event(type, x, y, extra));
const settled = () => new Promise(resolve => setImmediate(resolve));

test('only supported two-point primitives expose geometry handles; rich/text/number remain fixed', () => {
  assert.deepEqual(plain(geometry.drawingResizeHandles(shape())), [{ x: 140, y: 240 }, { x: 260, y: 240 }, { x: 260, y: 320 }, { x: 140, y: 320 }]);
  for (const kind of ['line', 'arrow']) assert.deepEqual(plain(geometry.drawingResizeHandles(shape(kind))), shape().points);
  for (const kind of ['rich', 'text', 'number', 'pen', 'highlighter', 'mosaic']) assert.deepEqual(geometry.drawingResizeHandles(shape(kind)), []);
  assert.deepEqual(geometry.drawingResizeHandles(shape('rect', [{ x: 1, y: 2 }])), []);
});

test('all four corners fix their diagonal source anchor, preserve orientation and prevent flipping', () => {
  const reversed = shape('ellipse', [{ x: 260, y: 320 }, { x: 140, y: 240 }]);
  const expected = [ [{ x: 260, y: 320 }, { x: 115, y: 215 }], [{ x: 310, y: 320 }, { x: 140, y: 215 }], [{ x: 310, y: 370 }, { x: 140, y: 240 }], [{ x: 260, y: 370 }, { x: 115, y: 240 }] ];
  const pointers = [{ x: 115, y: 215 }, { x: 310, y: 215 }, { x: 310, y: 370 }, { x: 115, y: 370 }];
  for (let handle = 0; handle < 4; handle++) assert.deepEqual(plain(geometry.resizedDrawingPoints(reversed, handle, pointers[handle], false, region)), expected[handle]);
  assert.deepEqual(plain(geometry.resizedDrawingPoints(shape(), 0, { x: 480, y: 490 }, false, region)), [{ x: 258, y: 318 }, { x: 260, y: 320 }]);
  assert.equal(geometry.resizedDrawingPoints(shape(), 4, { x: 1, y: 1 }, false, region), undefined);
});

test('Shift corner locks square/circle at the fixed anchor and clips one shared side at source edges', () => {
  const points = geometry.resizedDrawingPoints(shape('ellipse'), 2, { x: 490, y: 270 }, true, region);
  assert.deepEqual(plain(points), [{ x: 140, y: 240 }, { x: 400, y: 500 }]);
  assert.equal(points[1].x - points[0].x, points[1].y - points[0].y);
});

test('handle geometry remains ten CSS pixels with independent x/y source ratios', () => {
  const fixture = setup(), size = fixture.handleSize();
  assert.equal(size.x * fixture.svg.box.width / region.width, 10);
  assert.equal(size.y * fixture.svg.box.height / region.height, 10);
  fixture.svg.box = { ...fixture.svg.box, width: 320, height: 210 };
  const resized = fixture.handleSize();
  assert.equal(resized.x * 320 / region.width, 10); assert.equal(resized.y * 210 / region.height, 10);
});

test('real pointer handler previews many moves but submits once with exact source pixels and preserved provenance', async () => {
  const fixture = setup(); fixture.beginResize(event('pointerdown', 260, 320), 2);
  fire(fixture, 'pointermove', 300, 340); fire(fixture, 'pointermove', 321.5, 371.25);
  assert.equal(fixture.commands.length, 0);
  assert.deepEqual(plain(fixture.draft().points), [{ x: 140, y: 240 }, { x: 321.5, y: 371.25 }]);
  fire(fixture, 'pointerup', 321.5, 371.25); fire(fixture, 'pointerup', 400, 400); await settled();
  assert.equal(fixture.commands.length, 1); assert.equal(fixture.commands[0].type, 'update_drawing'); assert.equal(fixture.commands[0].expectedRevision, 17);
  assert.deepEqual(fixture.commands[0].drawing, { ...shape(), points: [{ x: 140, y: 240 }, { x: 321.5, y: 371.25 }] });
  assert.equal(fixture.window.count(), 0); assert.equal(fixture.svg.count(), 0); assert.equal(fixture.hasGesture(), false);
});

test('line/arrow edit each endpoint independently, including stationary Shift press and release', async () => {
  for (const kind of ['line', 'arrow']) for (const handle of [0, 1]) {
    const drawing = shape(kind), fixture = setup(drawing), start = drawing.points[handle], anchor = drawing.points[1 - handle];
    fixture.beginResize(event('pointerdown', start.x, start.y), handle);
    const moved = handle === 0 ? { x: 180, y: 340 } : { x: 340, y: 270 };
    fire(fixture, 'pointermove', moved.x, moved.y); fixture.shift(true);
    const expected = geometry.constrainedEnd(kind, anchor, moved, true, region);
    assert.deepEqual(plain(fixture.draft().points[handle]), plain(expected)); assert.deepEqual(plain(fixture.draft().points[1 - handle]), anchor);
    fixture.shift(false); assert.deepEqual(plain(fixture.draft().points[handle]), moved);
    fire(fixture, 'pointerup', moved.x, moved.y); await settled();
    assert.equal(fixture.commands.length, 1); assert.deepEqual(fixture.commands[0].drawing.points[1 - handle], anchor);
  }
});

test('Shift uses the requested outside-canvas ray before clipping, preserving the original 0.7.3 endpoint rule', async () => {
  for (const kind of ['line', 'arrow']) {
    const drawing = shape(kind, [{ x: 460, y: 250 }, { x: 480, y: 300 }]), fixture = setup(drawing);
    fixture.beginResize(event('pointerdown', 480, 300), 1);
    fire(fixture, 'pointermove', 760, 500, { shiftKey: true });
    // Raw dx300,dy250 is diagonal. Clamping first would falsely select the vertical ray.
    assert.deepEqual(plain(fixture.draft().points), [{ x: 460, y: 250 }, { x: 500, y: 290 }]);
    fire(fixture, 'pointerup', 760, 500, { shiftKey: true }); await settled();
    assert.deepEqual(fixture.commands[0].drawing.points, [{ x: 460, y: 250 }, { x: 500, y: 290 }]);
    const opposite = setup(shape(kind, [{ x: 480, y: 300 }, { x: 460, y: 250 }]));
    opposite.beginResize(event('pointerdown', 480, 300), 0); fire(opposite, 'pointerup', 760, 500, { shiftKey: true }); await settled();
    assert.deepEqual(opposite.commands[0].drawing.points, [{ x: 500, y: 290 }, { x: 460, y: 250 }]);
  }
});

test('an invalid final line/arrow endpoint cancels the whole gesture instead of committing the previous valid pointer', async () => {
  for (const kind of ['line', 'arrow']) for (const handle of [0, 1]) {
    const drawing = shape(kind), fixture = setup(drawing), start = drawing.points[handle], anchor = drawing.points[1 - handle];
    fixture.beginResize(event('pointerdown', start.x, start.y), handle);
    fire(fixture, 'pointermove', 360, 390); assert.ok(fixture.draft());
    fire(fixture, 'pointerup', anchor.x + .25, anchor.y + .25); await settled();
    assert.equal(fixture.commands.length, 0); assert.equal(fixture.draft(), undefined); assert.equal(fixture.hasGesture(), false);
    assert.deepEqual(fixture.props.region.drawings[0], drawing); assert.equal(fixture.window.count(), 0); assert.equal(fixture.svg.count(), 0);
  }
});

test('press/release, returning to original geometry and another pointer produce no write', async () => {
  for (const mode of ['click', 'return', 'other']) {
    const fixture = setup(); fixture.beginResize(event('pointerdown', 260, 320), 2);
    if (mode === 'return') fire(fixture, 'pointermove', 330, 370);
    if (mode === 'other') { fire(fixture, 'pointermove', 330, 370, { pointerId: 9 }); fire(fixture, 'pointerup', 330, 370, { pointerId: 9 }); }
    fire(fixture, 'pointerup', 260, 320); await settled(); assert.equal(fixture.commands.length, 0); assert.equal(fixture.hasGesture(), false);
  }
});

test('pointercancel, lostcapture, blur/flush cancellation, unmount and released-button move discard previews', async () => {
  for (const mode of ['pointercancel', 'lostpointercapture', 'blur', 'resize', 'flush', 'dispose', 'buttons']) {
    const fixture = setup(); fixture.beginResize(event('pointerdown', 260, 320), 2); fire(fixture, 'pointermove', 330, 370);
    if (mode === 'pointercancel') fire(fixture, mode, 330, 370);
    else if (mode === 'lostpointercapture') fixture.svg.dispatch(event(mode, 330, 370));
    else if (mode === 'flush') await fixture.flush();
    else if (mode === 'dispose') fixture.dispose();
    else if (mode === 'blur') fixture.blur();
    else if (mode === 'resize') fixture.resize();
    else if (mode === 'buttons') fire(fixture, 'pointermove', 340, 380, { buttons: 0 });
    fire(fixture, 'pointerup', 330, 370); await settled();
    assert.equal(fixture.commands.length, 0, mode); assert.equal(fixture.draft(), undefined, mode); assert.equal(fixture.window.count(), 0, mode); assert.equal(fixture.svg.count(), 0, mode);
  }
});

test('real Solid source fence cancels on revision/source change but survives unrelated snapshot publication', async () => {
  const solid = await import('solid-js/dist/solid.js');
  const effect = body.find(node => node.type === 'ExpressionStatement' && node.expression.type === 'CallExpression' && node.expression.callee.name === 'createEffect' && node.expression.arguments[0]?.type === 'CallExpression' && node.expression.arguments[0].arguments[0]?.name === 'gestureSource');
  assert.ok(effect);
  const product = new SourceTextModule(transform(`import { drawingDraftKey, richSourceIdentity } from 'dependencies';
    export function fence(props, createMemo, createEffect, on, cancel) { let cancelGesture = cancel, eraseSource;
      ${declaration('cacheKey')} ${declaration('gestureIdentity')} ${declaration('gestureSource')} ${code.slice(effect.start, effect.end)}
    }`));
  await product.link(() => synthetic({ drawingDraftKey: properties.drawingDraftKey, richSourceIdentity: preview.richSourceIdentity })); await product.evaluate();
  const baseline = setup().props, [state, setState] = solid.createSignal(baseline);
  let reactivePort; const props = new Proxy({}, { get: (_, key) => key === "port" ? reactivePort : state()[key] }); reactivePort=imageMod.namespace.imagePort(props);
  let canceled = 0, dispose;
  solid.createRoot(cleanup => { dispose = cleanup; product.namespace.fence(props, solid.createMemo, solid.createEffect, solid.on, () => canceled++); });
  await settled(); const initial = canceled;
  setState({ ...state(), unrelatedDraft: 'new message' }); await settled(); assert.equal(canceled, initial);
  setState({ ...state(), region: { ...state().region, drawingRevision: 18 } }); await settled(); assert.equal(canceled, initial + 1);
  setState({ ...state(), background: { ...state().background, path: 'assets/changed.png' } }); await settled(); assert.equal(canceled, initial + 2);
  dispose();
});

test('revision, full source identity, crop, DOM mapping, busy and tool changes cannot submit a stale gesture', async () => {
  const changes = [ f => f.props.region.drawingRevision++, f => f.props.background.path = 'assets/new.png', f => f.props.background.scaleFactor = 1.5,
    f => f.props.region.imageOverride = { ...f.props.background, id: 'override' }, f => f.props.region.x++, f => f.props.backgroundWidth++, f => f.props.box.x++,
    f => f.svg.box.width++, f => f.svg.box.top++, f => f.props.busy = true, f => f.tool('pen'), f => f.edit({}) ];
  for (const change of changes) {
    const fixture = setup(); fixture.beginResize(event('pointerdown', 260, 320), 2); fire(fixture, 'pointermove', 330, 370); change(fixture);
    fire(fixture, 'pointerup', 340, 380); await settled(); assert.equal(fixture.commands.length, 0); assert.equal(fixture.draft(), undefined);
  }
});

test('accepted resize stays single-flight; rejected CAS leaves original shape and can be retried', async () => {
  let reject; const waiting = new Promise((_, no) => reject = no); const fixture = setup(shape(), () => waiting);
  fixture.beginResize(event('pointerdown', 260, 320), 2); fire(fixture, 'pointerup', 330, 370);
  assert.equal(fixture.pending(), true); fixture.beginResize(event('pointerdown', 260, 320), 2); assert.equal(fixture.hasGesture(), false); assert.equal(fixture.commands.length, 1);
  reject(new Error('CAS conflict')); assert.equal(await fixture.flush(), false); await settled();
  assert.equal(fixture.pending(), false); assert.equal(fixture.draft(), undefined); assert.deepEqual(fixture.props.region.drawings[0], shape()); assert.deepEqual(fixture.errors, ['CAS conflict']);
  fixture.props.onCommand = command => { fixture.commands.push(plain(command)); return Promise.resolve(); };
  fixture.beginResize(event('pointerdown', 260, 320), 2); fire(fixture, 'pointerup', 340, 380); await settled(); assert.equal(fixture.commands.length, 2);
});

test('fixed rich identity and document-only mode cannot enter resize even through direct event invocation', () => {
  for (const kind of ['rich', 'text', 'number', 'pen', 'highlighter', 'mosaic']) {
    const fixture = setup(shape(kind)); fixture.beginResize(event('pointerdown', 260, 320), 0); assert.equal(fixture.hasGesture(), false); assert.equal(fixture.commands.length, 0);
  }
  const fixture = setup(); fixture.props.documentOnly = true; fixture.beginResize(event('pointerdown', 260, 320), 2); assert.equal(fixture.hasGesture(), false);
});
