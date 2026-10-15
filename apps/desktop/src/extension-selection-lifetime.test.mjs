// SPDX-License-Identifier: MPL-2.0
// Mount the production toolbar's Show condition/blur handlers and complete
// ExtensionSelectionTools JSX with Solid's real renderer. Native IPC is only a
// deferred boundary; no region/LLM/desktop-operation Host is simulated here.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const ts = source => stripTypeScriptTypes(source, { mode: 'transform' });
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; };
const canvasSource = await raw('./components/SpaceCanvas.tsx');
const nodes = [];
function walk(node) {
  if (!node || typeof node !== 'object') return;
  if (node.type) nodes.push(node);
  for (const [key, value] of Object.entries(node)) {
    if (key === 'loc') continue;
    if (Array.isArray(value)) value.forEach(walk); else if (value && typeof value === 'object') walk(value);
  }
}
walk(parse(canvasSource, { sourceType: 'module', plugins: ['typescript', 'jsx'] }));
const element = nodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'ExtensionSelectionTools');
const toolbar = nodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'Show'
  && node.start < element.start && node.end > element.end && node.openingElement.attributes.some(attribute => attribute.name?.name === 'when'
    && canvasSource.slice(attribute.value.expression.start, attribute.value.expression.end).includes('toolbarVisible()')));
const when = toolbar.openingElement.attributes.find(attribute => attribute.name?.name === 'when').value.expression;
const actualVariable = name => {
  const node = nodes.find(node => node.type === 'VariableDeclarator' && (node.id.name === name
    || node.id.type === 'ArrayPattern' && node.id.elements.some(value => value?.name === name)));
  assert.ok(node, `production variable ${name}`); return `const ${canvasSource.slice(node.start, node.end)};`;
};
const actualFunction = name => {
  const node = nodes.find(node => node.type === 'FunctionDeclaration' && node.id.name === name);
  assert.ok(node, `production handler ${name}`); return canvasSource.slice(node.start, node.end);
};
// Geometry, drawing and unrelated toolbar buttons are absent from this bounded
// ownership fixture. The production condition, IPC call site, handlers and child
// JSX remain executable source, rather than a rewritten visibility controller.
const harness = `import { createSignal, Show, onCleanup } from 'solid-js';
import ExtensionSelectionTools from './ExtensionSelectionTools';
export default function ToolbarOwner(props) {
  ${['toolbarVisible', 'pluginMenu', 'extensionButtonCount', 'extensionInteracting', 'extensionRunning', 'blocked'].map(actualVariable).join('\n')}
  let hideTimer, cancelDeferredPointer, cancelGesture;
  const activeRegion = () => props.scene.regions[0], activity = () => false, forceNew = () => false;
  const drawingSession = () => props.drawing, exporting = () => false, settling = () => false;
  const setCodeFocused = () => {}, clearSnapPreview = () => {}, endNudge = () => {};
  ${['clearHideTimer', 'hideToolbar', 'scheduleHide'].map(actualFunction).join('\n')}
  ${actualVariable('blur')}
  Object.assign(props.control, { blur, hideToolbar, leave: scheduleHide, reveal: () => setToolbarVisible(true) });
  onCleanup(clearHideTimer);
  return <Show when={${canvasSource.slice(when.start, when.end)}}>{region => <div class="capture-toolbar">
    ${canvasSource.slice(element.start, element.end)}
  </div>}</Show>;
}`;

async function fixture({ draft = 'Click the test control', multiple = false } = {}) {
  const request = deferred(), calls = [], stops = [], errors = [], listeners = new Set(), control = {};
  let subscriptions = 0, released = 0;
  const context = createContext({ console, queueMicrotask, setTimeout, clearTimeout });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const core = synthetic(solid);
  const universal = new SourceTextModule(await raw('../../../node_modules/solid-js/universal/dist/universal.js'), { context });
  await universal.link(() => core); await universal.evaluate();
  const renderer = universal.namespace.createRenderer({
    createElement: type => ({ type, children: [], props: {}, parent: null }), createTextNode: text => ({ text, children: [], parent: null }),
    replaceText: (node, text) => { node.text = text; }, setProperty: (node, key, value) => { node.props[key] = value; },
    insertNode: (parent, node, anchor) => { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); const index = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(index < 0 ? parent.children.length : index, 0, node); node.parent = parent; },
    removeNode: (parent, node) => { parent.children.splice(parent.children.indexOf(node), 1); node.parent = null; },
    isTextNode: node => 'text' in node, getParentNode: node => node.parent, getFirstChild: node => node.children[0], getNextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1],
  });
  const rendererModule = synthetic(renderer);
  const service = { kind: 'command', id: 'operate', title: '区域操作', metadata: { placement: 'selection-toolbar' } };
  const installed = { manifest: { id: 'independent.region', version: '1.0.0' }, revision: 1, state: 'enabled', services: multiple ? [service, { ...service, id: 'second', title: 'Second' }] : [service] };
  const native = synthetic({ extensionNative: true,
    subscribeExtensions: async callback => { subscriptions++; listeners.add(callback); return () => { released++; listeners.delete(callback); }; },
    getExtensions: async () => [installed],
    invokeExtension: invocation => { calls.push(invocation); control.blur(); return request.promise; },
    stopExtension: async id => { stops.push(id); },
  });
  const state = new SourceTextModule(ts(await raw('./extension-state.ts')), { context }); await state.link(() => { throw Error('Unexpected state import'); }); await state.evaluate();
  const icons = synthetic(Object.fromEntries(['LoaderCircle', 'Puzzle', 'Square', 'X'].map(name => [name, () => renderer.createElement('svg')])));
  const compile = source => ts(transformSync(source, { filename: 'selection-lifetime.tsx', babelrc: false, configFile: false,
    parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]] }).code);
  const child = new SourceTextModule(compile(await raw('./components/ExtensionSelectionTools.tsx')), { context });
  await child.link(name => name === 'solid-js' ? core : name === 'test-renderer' ? rendererModule : name === 'lucide-solid' ? icons
    : name === '../extension-bridge' ? native : name === '../extension-state' ? state : name === '../i18n' ? synthetic({ t: value => value }) : synthetic({}));
  await child.evaluate();
  const owner = new SourceTextModule(compile(harness), { context });
  await owner.link(name => name === 'solid-js' ? core : name === 'test-renderer' ? rendererModule : child); await owner.evaluate();
  const [scene, setScene] = solid.createSignal({ id: 'scene', draft, regions: [{ id: 'region' }] });
  const [drawing, setDrawing] = solid.createSignal(undefined);
  const root = renderer.createElement('root');
  const dispose = renderer.render(() => owner.namespace.default({ get scene() { return scene(); }, get drawing() { return drawing(); }, busy: false, control, onError: error => errors.push(error) }), root);
  const all = () => { const values = []; const visit = node => { values.push(node); node.children.forEach(visit); }; visit(root); return values; };
  const find = predicate => all().find(predicate);
  const button = label => find(node => node.type === 'button' && node.props['aria-label'] === label);
  const text = node => 'text' in node ? node.text : node.children.map(text).join('');
  const toolbarNode = () => find(node => node.props?.class === 'capture-toolbar');
  control.reveal(); await tick();
  return { control, all, find, text, button, toolbarNode, request, calls, stops, errors, setScene, setDrawing, dispose,
    get subscriptions() { return subscriptions; }, get released() { return released; }, get listenerCount() { return listeners.size; },
    typeTask: value => { const input = find(node => node.type === 'textarea'); assert.ok(input, 'task editor remains mounted'); input.props.onInput({ currentTarget: { value } }); },
    submit: () => { const form = find(node => node.type === 'form'); assert.ok(form, 'task form remains mounted'); form.props.onSubmit({ preventDefault() {} }); },
  };
}

test('observation blur retains the actual toolbar owner, Stop button, listener and completed result', async () => {
  const f = await fixture();
  try {
    const toolbar = f.toolbarNode(); f.button('选区插件').props.onClick(); await tick();
    assert.equal(f.calls.length, 1); assert.deepEqual(f.stops, [], 'native observation blur must not cancel its own invocation');
    assert.equal(f.toolbarNode(), toolbar); assert.ok(f.button('停止插件执行')); assert.equal(f.subscriptions, 1); assert.equal(f.released, 0);
    f.control.blur(); f.control.leave(); assert.equal(f.toolbarNode(), toolbar);
    f.request.resolve({ content: 'Verified by the following screenshot' }); await tick();
    assert.ok(f.find(node => node.props?.role === 'status')); assert.ok(f.all().some(node => 'text' in node && node.text === 'Verified by the following screenshot'));
    assert.deepEqual(f.stops, []); assert.equal(f.toolbarNode(), toolbar);
    f.button('关闭').props.onClick(); assert.equal(f.toolbarNode(), undefined); assert.equal(f.released, 1);
  } finally { f.dispose(); }
});

test('a blurred empty-draft task editor hands off to running without an intermediate owner disposal', async () => {
  const f = await fixture({ draft: '' });
  try {
    f.button('选区插件').props.onClick(); const form = f.find(node => node.type === 'form'); assert.ok(form);
    f.control.blur(); assert.equal(f.find(node => node.type === 'form'), form);
    f.typeTask('Click only the test button'); f.submit(); await tick();
    assert.equal(f.calls.length, 1); assert.equal(f.calls[0].input.task, 'Click only the test button'); assert.equal(f.calls[0].sceneId, 'scene'); assert.equal(f.calls[0].regionId, 'region');
    assert.deepEqual(f.stops, []); assert.equal(f.subscriptions, 1); assert.equal(f.released, 0); assert.ok(f.button('停止插件执行'));
    f.request.resolve({ content: 'Done' }); await tick(); assert.ok(f.find(node => node.props?.role === 'status'));
  } finally { f.dispose(); }
});

test('same-ID deep-cloned snapshots retain the running invocation and publish its result', async () => {
  const f = await fixture();
  try {
    f.button('选区插件').props.onClick(); await tick();
    const toolbar = f.toolbarNode(), stop = f.button('停止插件执行'); assert.ok(stop);
    for (let revision = 1; revision <= 2; revision++) {
      f.setScene(previous => ({ ...structuredClone(previous), revision })); await tick();
      assert.deepEqual(f.stops, [], 'a refreshed snapshot with the same primitive IDs is not a cancellation boundary');
      assert.equal(f.toolbarNode(), toolbar); assert.equal(f.button('停止插件执行'), stop);
      assert.equal(f.subscriptions, 1); assert.equal(f.released, 0); assert.equal(f.calls.length, 1);
    }
    f.request.resolve({ content: 'Result survives the refreshed snapshot' }); await tick();
    assert.ok(f.find(node => node.props?.role === 'status'));
    assert.ok(f.all().some(node => 'text' in node && node.text === 'Result survives the refreshed snapshot'));
    assert.deepEqual(f.errors, []);
  } finally { f.dispose(); }
});

test('same-ID deep-cloned snapshots retain an open command menu and typed task editor', async () => {
  const f = await fixture({ draft: '', multiple: true });
  try {
    f.button('选区插件').props.onClick(); f.control.blur();
    const menu = f.find(node => node.props?.role === 'menu'); assert.ok(menu);
    f.setScene(previous => structuredClone(previous)); await tick();
    assert.equal(f.find(node => node.props?.role === 'menu'), menu);
    f.find(node => node.props?.role === 'menuitem').props.onClick();
    f.typeTask('Keep this task while the scene snapshot refreshes');
    const form = f.find(node => node.type === 'form'), input = f.find(node => node.type === 'textarea');
    for (let revision = 1; revision <= 2; revision++) {
      f.setScene(previous => ({ ...structuredClone(previous), revision })); await tick();
      assert.equal(f.find(node => node.type === 'form'), form); assert.equal(f.find(node => node.type === 'textarea'), input);
      assert.equal(input.props.value, 'Keep this task while the scene snapshot refreshes');
      assert.equal(f.subscriptions, 1); assert.equal(f.released, 0); assert.deepEqual(f.stops, []);
    }
    f.submit(); await tick();
    assert.equal(f.calls.length, 1); assert.equal(f.calls[0].input.task, 'Keep this task while the scene snapshot refreshes');
    f.request.resolve({ content: 'Done' }); await tick();
  } finally { f.dispose(); }
});

test('same-ID deep-cloned snapshots retain the completed result until an actual region change', async () => {
  const f = await fixture();
  try {
    f.button('选区插件').props.onClick(); await tick(); f.request.resolve({ content: 'Keep the completed result' }); await tick();
    const status = f.find(node => node.props?.role === 'status'), toolbar = f.toolbarNode(); assert.ok(status);
    for (let revision = 1; revision <= 2; revision++) {
      f.setScene(previous => ({ ...structuredClone(previous), revision })); await tick();
      assert.equal(f.find(node => node.props?.role === 'status'), status); assert.equal(f.toolbarNode(), toolbar);
      assert.equal(f.subscriptions, 1); assert.equal(f.released, 0); assert.deepEqual(f.stops, []);
    }
    f.setScene(previous => ({ ...structuredClone(previous), regions: [{ id: 'other-region' }] })); await tick();
    assert.equal(f.find(node => node.props?.role === 'status'), undefined, 'a genuinely different region clears the previous result');
  } finally { f.dispose(); }
});

test('blurred multi-command menus hand off to task input and direct execution without losing the owner', async () => {
  for (const draft of ['', 'Click only the test button']) {
    const f = await fixture({ draft, multiple: true });
    try {
      f.button('选区插件').props.onClick(); assert.ok(f.find(node => node.props?.role === 'menu')); f.control.blur();
      const option = f.find(node => node.props?.role === 'menuitem'); assert.ok(option); option.props.onClick(); await tick();
      assert.equal(f.released, 0); assert.equal(f.subscriptions, 1);
      if (!draft) { assert.ok(f.find(node => node.type === 'form')); f.typeTask('Click only the test button'); f.submit(); await tick(); }
      assert.equal(f.calls.length, 1); assert.ok(f.button('停止插件执行')); assert.deepEqual(f.stops, []);
      f.request.resolve({ content: 'Done' }); await tick();
    } finally { f.dispose(); }
  }
});

test('ordinary hover departure still hides after its delay; explicit drawing removes an idle task editor', async () => {
  const f = await fixture({ draft: '' });
  try {
    f.control.leave(); await new Promise(resolve => setTimeout(resolve, 330));
    assert.equal(f.toolbarNode(), undefined); assert.equal(f.released, 1); assert.deepEqual(f.stops, []);
    f.control.reveal(); await tick(); f.button('选区插件').props.onClick(); assert.ok(f.find(node => node.type === 'form'));
    f.control.hideToolbar(); f.setDrawing({ regionId: 'region' });
    assert.equal(f.toolbarNode(), undefined); assert.equal(f.released, 2); assert.equal(f.calls.length, 0);
  } finally { f.dispose(); }
});

test('scene/region changes, region removal and real owner disposal still stop and reject late results', async () => {
  for (const boundary of ['scene', 'region', 'remove', 'dispose']) {
    const f = await fixture();
    try {
      f.button('选区插件').props.onClick(); await tick(); assert.deepEqual(f.stops, []);
      if (boundary === 'dispose') f.dispose();
      else f.setScene(previous => boundary === 'scene' ? { ...previous, id: 'other-scene' }
        : { ...previous, regions: boundary === 'region' ? [{ id: 'other-region' }] : [] });
      await tick(); assert.equal(f.stops.length, 1, `${boundary} cancels the actual invocation`);
      f.request.resolve({ content: 'Must never appear' }); await tick();
      assert.equal(f.all().some(node => 'text' in node && node.text === 'Must never appear'), false);
    } finally { f.dispose(); }
  }
});
