// SPDX-License-Identifier: MPL-2.0
// Actual production menu, bridge functions and source/flush guards; no native file picker or user data.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule, createContext, runInContext } from 'node:vm';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const plain = source => stripTypeScriptTypes(source, { mode: 'transform' });
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const universal = new SourceTextModule(await raw('../../../node_modules/solid-js/universal/dist/universal.js'));
await universal.link(() => synthetic(solid)); await universal.evaluate();
let active, events;
const walk = (node, predicate) => [...(predicate(node) ? [node] : []), ...node.children.flatMap(child => walk(child, predicate))];
const renderer = universal.namespace.createRenderer({
  createElement: tag => ({ tag, props: {}, children: [], isConnected: true, offsetWidth: 148, offsetHeight: 82, contains(node) { return node === this || this.children.some(child => child.contains?.(node)); }, focus() { active = this; this.props.onFocus?.(); }, getBoundingClientRect: () => ({ left: 40, top: 50, right: 88, bottom: 98, width: 48, height: 48 }) }),
  createTextNode: text => ({ text, children: [] }), replaceText: (node, text) => node.text = text, isTextNode: node => 'text' in node,
  setProperty: (node, name, value) => node.props[name] = value,
  insertNode: (parent, node, anchor) => { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); node.parent = parent; const index = anchor ? parent.children.indexOf(anchor) : -1; if (index < 0) parent.children.push(node); else parent.children.splice(index, 0, node); },
  removeNode: (parent, node) => { parent.children.splice(parent.children.indexOf(node), 1); node.parent = undefined; },
  getParentNode: node => node.parent, getFirstChild: node => node.children[0], getNextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1],
});
const code = transformSync(await raw('./components/ImageExportButton.tsx'), { filename: 'image-export.tsx', configFile: false, babelrc: false, parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]] }).code;
const component = new SourceTextModule(plain(code));
await component.link(name => synthetic(name === 'test-renderer' ? renderer : name === 'solid-js' ? solid : name === 'solid-js/web' ? { Portal: props => props.children } : name === '../i18n' ? { t: value => value } : name === 'lucide-solid' ? { Download: () => renderer.createElement('svg') } : {}));
await component.evaluate();
const tick = () => new Promise(resolve => setImmediate(resolve));
function fixture() {
  const previous = { window: globalThis.window, innerWidth: globalThis.innerWidth, innerHeight: globalThis.innerHeight };
  events = new Map(); globalThis.window = { addEventListener(type, handler) { const rows = events.get(type) ?? new Set(); rows.add(handler); events.set(type, rows); }, removeEventListener(type, handler) { events.get(type)?.delete(handler); } };
  globalThis.innerWidth = 800; globalThis.innerHeight = 600;
  const [scope, setScope] = solid.createSignal('scene/region/background'), [disabled, setDisabled] = solid.createSignal(false);
  const root = renderer.createElement('root'), selected = [], openChanges = []; let registered;
  const dispose = renderer.render(() => renderer.createComponent(component.namespace.default, { get scope() { return scope(); }, get disabled() { return disabled(); }, onSelect: value => selected.push(value), onOpenChange: value => openChanges.push(value), onRegister: value => { registered = value; return () => { registered = undefined; }; } }), root);
  const buttons = () => walk(root, node => node.tag === 'button');
  return { root, selected, openChanges, setScope, setDisabled, trigger: () => buttons()[0], items: () => buttons().filter(node => node.props.role === 'menuitem'), open: () => registered?.(), emit(type, fields = {}) { const event = { ...fields, preventDefault() { this.prevented = true; }, stopImmediatePropagation() { this.stopped = true; } }; for (const handler of [...(events.get(type) ?? [])]) handler(event); return event; }, click() { this.trigger().props.onClick({ stopPropagation() {} }); }, close() { dispose(); Object.assign(globalThis, previous); } };
}

test('production save trigger opens exactly two choices and dispatches only the chosen variant', async () => {
  const f = fixture();
  try {
    f.click(); await tick(); assert.deepEqual(f.selected, []); assert.equal(f.trigger().props['aria-expanded'], true);
    assert.deepEqual(f.items().map(node => node.children.map(child => child.text ?? '').join('')), ['带标注图', '原图']);
    assert.equal(active, f.items()[0]);
    const down = f.emit('keydown', { key: 'ArrowDown' }); assert.ok(down.prevented && down.stopped); assert.equal(active, f.items()[1]);
    f.emit('keydown', { key: 'Enter' }); assert.deepEqual(f.selected, ['original']); assert.equal(active, f.trigger()); assert.equal(f.items().length, 0);
    f.click(); await tick(); const item = f.items()[0]; item.props.onClick(); item.props.onClick();
    assert.deepEqual(f.selected, ['original', 'annotated'], 'removed menu cannot issue a duplicate save');
  } finally { f.close(); }
});

test('menu owns Escape before capture/editor exit and closes on outside input, source change, lock, blur and disposal', async () => {
  const f = fixture();
  try {
    f.open(); await tick(); const escape = f.emit('keydown', { key: 'Escape' });
    assert.ok(escape.prevented && escape.stopped); assert.equal(f.items().length, 0); assert.equal(active, f.trigger());
    for (const action of ['outside', 'source', 'disabled', 'blur', 'resize', 'tab']) {
      f.setDisabled(false); f.open(); await tick(); const stale = f.items()[0]; assert.ok(stale, action);
      if (action === 'outside') { const event = f.emit('pointerdown', { target: {} }); assert.ok(!event.stopped); }
      if (action === 'source') f.setScope('other/source');
      if (action === 'disabled') f.setDisabled(true);
      if (action === 'blur' || action === 'resize') f.emit(action);
      if (action === 'tab') assert.ok(!f.emit('keydown', { key: 'Tab' }).prevented);
      assert.equal(f.items().length, 0, action); stale.props.onClick(); assert.deepEqual(f.selected, [], action);
    }
    f.setDisabled(false); f.open(); await tick(); const stale = f.items()[0];
    f.close(); stale.props.onClick(); assert.deepEqual(f.selected, []); assert.ok([...events.values()].every(value => value.size === 0));
  } finally { if (globalThis.window !== undefined && events.get('keydown')?.size) f.close(); }
});

test('disabled trigger never opens and keyboard opening, Home/End and bounds use the same menu', async () => {
  const f = fixture();
  try {
    f.setDisabled(true); f.open(); f.click(); assert.equal(f.items().length, 0);
    f.setDisabled(false); f.trigger().props.onKeyDown({ key: 'ArrowUp', preventDefault() {}, stopPropagation() {} }); await tick();
    assert.equal(active, f.items()[1]); f.emit('keydown', { key: 'Home' }); assert.equal(active, f.items()[0]); f.emit('keydown', { key: 'End' }); assert.equal(active, f.items()[1]);
    f.emit('keydown', { key: ' ' }); assert.deepEqual(f.selected, ['original']);
  } finally { f.close(); }
});

function declarations(source, names, component = false) {
  const body = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] }).program.body;
  const rows = component ? body.find(node => node.type === 'ExportDefaultDeclaration').declaration.body.body : body.map(node => node.type === 'ExportNamedDeclaration' ? node.declaration : node);
  return names.map(name => { const node = rows.find(node => node?.type === 'FunctionDeclaration' && node.id?.name === name); assert.ok(node, name); return plain(source.slice(node.start, node.end)); }).join('\n');
}

test('actual image IPC forwards typed explicit variants, forces annotated clipboard, and rejects invalid input/result', async () => {
  const calls = [], context = createContext({ native: true, invoke: async (command, args) => { calls.push({ command, args: structuredClone(args) }); return true; } });
  runInContext(declarations(await raw('./bridge.ts'), ['exportRegion', 'exportImageItem']), context);
  await context.exportRegion('scene', 'region', false, 'original'); await context.exportRegion('scene', 'region', false, 'annotated');
  await context.exportRegion('scene', 'region', true, 'original'); await context.exportImageItem('scene', 'item', 'asset', false, 'original');
  await context.exportImageItem('scene', 'item', 'asset', true); await context.exportRegion('scene', 'region', false);
  assert.deepEqual(calls.map(value => value.args.variant), ['original', 'annotated', 'annotated', 'original', 'annotated', 'annotated']);
  assert.deepEqual(calls[3].args, { sceneId: 'scene', itemId: 'item', sourceAssetId: 'asset', clipboard: false, variant: 'original' });
  await assert.rejects(context.exportRegion('scene', 'region', false, 'clean')); await assert.rejects(context.exportImageItem('scene', 'item', 'asset', false, 'file:///bad'));
  assert.equal(calls.length, 6); context.invoke = async () => ({}); await assert.rejects(context.exportRegion('scene', 'region', false, 'original'));
});

const completion = new SourceTextModule(plain(await raw('./capture-completion.ts'))); await completion.link(() => { throw Error('Unexpected dependency'); }); await completion.evaluate();
test('actual region/object export waits for complete authoring flush, preserves variant and rejects changed sources', async () => {
  const actual = declarations(await raw('./components/SpaceCanvas.tsx'), ['exportRegion', 'exportImage'], true);
  for (const kind of ['region', 'image']) {
    const selected = [], scene = { id: 'scene', background: { id: 'background' }, regions: [{ id: 'region', x: 10, y: 20, width: 300, height: 200, drawingRevision: 0 }], items: [{ id: 'item', asset: { id: 'asset' } }] };
    let release, busy = false; const flush = new Promise(resolve => release = resolve);
    const context = createContext({ JSON, disposed: false, native: true, finishCapture: completion.namespace.finishCapture, props: { scene, busy: false, onBeforeExport: () => flush, onError: message => { throw Error(message); } }, exporting: () => busy, setExporting: value => busy = value, exportNativeRegion: async (...args) => { selected.push(args); return true; }, exportImageItem: async (...args) => { selected.push(args); return true; } });
    runInContext(actual, context);
    const saving = kind === 'region' ? context.exportRegion(scene.regions[0], false, false, 'original') : context.exportImage(scene.items[0], false, false, 'original');
    assert.equal(selected.length, 0); release(); assert.equal(await saving, true); assert.equal(selected[0].at(-1), 'original');
    const before = selected.length; context.props.onBeforeExport = async () => { context.props.scene = { ...scene, id: 'other' }; };
    const canceled = kind === 'region' ? context.exportRegion(scene.regions[0], false, false, 'original') : context.exportImage(scene.items[0], false, false, 'original');
    assert.equal(await canceled, false); assert.equal(selected.length, before);
  }
});

test('actual shared drawing save drains strokes/history and text before selected export without leaving the editor', async () => {
  const events = [], context = createContext({ disposed: false, cancelGesture: undefined, disabled: () => false, historyDisabled: () => false, pending: () => false, finishing: () => false, setFinishing() {}, identity: () => 'same-scene', strokeQueue: { drain: async () => events.push('strokes') }, drainHistory: async () => events.push('history'), saveText: async () => { events.push('text'); return true; }, finishDrawing: completion.namespace.finishDrawing, props: { onExport: async (copy, close, variant) => { events.push([copy, close, variant]); return true; }, onClose: () => events.push('close') }, report: value => { throw value; } });
  runInContext(declarations(await raw('./components/SharedDrawingEditor.tsx'), ['done'], true), context);
  await context.done(false, false, false, 'original');
  assert.deepEqual(events, ['strokes', 'history', 'text', [false, false, 'original']]);
});
