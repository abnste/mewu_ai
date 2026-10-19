// SPDX-License-Identifier: MPL-2.0
// Actual production ScrollSurface/ScrollBackdrop with Solid's reactive runtime.
// Host events and DOM are synthetic; no screen, global shortcut or user data.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule, createContext } from 'node:vm';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import postcss from 'postcss';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const tick = () => new Promise(resolve => setImmediate(resolve));
const plain = value => JSON.parse(JSON.stringify(value));
const source = await raw('./ScrollSurface.tsx');
const compiled = stripTypeScriptTypes(transformSync(source, { filename: 'ScrollSurface.tsx', configFile: false, babelrc: false,
  parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]] }).code, { mode: 'transform' });
async function fixture() {
  const keys = new Map(), calls = []; let listener, stopped = 0, disposed;
  const document = { documentElement: { dataset: {} }, addEventListener: (name, fn) => keys.set(name, fn), removeEventListener: (name, fn) => { if (keys.get(name) === fn) keys.delete(name); } };
  const context = createContext({ document, console, Promise });
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
  const bridge = new SourceTextModule(stripTypeScriptTypes(await raw('./scroll-bridge.ts'), { mode: 'transform' }), { context });
  await bridge.link(name => name.endsWith('/core') ? synthetic({ isTauri: () => false, invoke: () => { throw Error('No fixture IPC'); } }) : synthetic({ listen: () => { throw Error('No fixture native events'); } })); await bridge.evaluate();
  const module = new SourceTextModule(compiled, { context });
  await module.link(name => {
    if (name === 'solid-js') return core;
    if (name === 'test-renderer') return synthetic(renderer);
    if (name === './i18n') return synthetic({ t: value => value });
    if (name === './native-select-escape') return synthetic({ nativeSelectOwnsEscape: () => false });
    if (name === './scroll-bridge') return synthetic({ scrollNeedsKeep: bridge.namespace.scrollNeedsKeep, subscribeScroll: async (next, surface) => { assert.equal(surface, 'scroll-controls'); listener = next; return () => stopped++; }, controlScrollCapture: async (id, action) => { calls.push({ id, action }); } });
    if (name === 'lucide-solid') return synthetic({ Check: () => null, CircleAlert: () => null, LoaderCircle: () => null, X: () => null });
    if (name.endsWith('.css')) return synthetic({});
    throw Error(`Unexpected import ${name}`);
  }); await module.evaluate();
  const root = { children: [] };
  solid.createRoot(dispose => { disposed = dispose; renderer.insert(root, () => module.namespace.default()); });
  await tick();
  const nodes = () => { const result = []; const walk = node => { result.push(node); node.children?.forEach(walk); }; walk(root); return result; };
  const one = key => nodes().find(node => node.props?.class === key);
  return { next: value => listener(value), nodes, one, calls, keys, module, renderer, document, dispose: () => disposed(), stopped: () => stopped };
}
const initial = { id: 'capture-a', sceneId: 'synthetic', phase: 'capturing', rect: { x: .3, y: .25, width: .4, height: .5 }, width: 600, height: 400, status: 'initial', stopHotkey: 'F8 / Esc', preview: 'data:image/png;base64,first',
  hud: { outline: { x: .299, y: .248, width: .402, height: .504 }, preview: { x: .15, y: .45, width: .12, height: .3 }, controls: { x: .35, y: .1, width: .2, height: .08 } } };

test('source-monitor outline and growing accepted preview stay mounted while image height and image pixels update', async () => {
  const f = await fixture(); f.next(initial); await tick();
  const outline = f.one('scroll-live-outline'), preview = f.one('scroll-preview'), controls = f.one('scroll-controls-slot');
  assert.ok(outline && preview && controls);
  assert.deepEqual(plain(outline.props.style), { left: '29.9%', top: '24.8%', width: '40.2%', height: '50.4%' });
  assert.deepEqual(plain(preview.props.style), { left: '15%', top: '45%', width: '12%', height: '30%' });
  assert.deepEqual(plain(controls.props.style), { left: '35%', top: '10%', width: '20%', height: '8%' });
  const image = f.nodes().find(node => node.type === 'img');
  f.next({ ...initial, height: 800, status: 'extended', preview: 'data:image/png;base64,second', hud: { ...initial.hud, preview: { ...initial.hud.preview, y: .15, height: .6 } } }); await tick();
  assert.equal(f.one('scroll-preview'), preview); assert.equal(f.one('scroll-controls-slot'), controls); assert.equal(f.one('scroll-live-outline'), outline);
  assert.equal(f.nodes().find(node => node.type === 'img'), image);
  assert.equal(image.props.src, 'data:image/png;base64,second'); assert.equal(preview.props.style.height, '60%'); assert.equal(preview.props.style.top, '15%');
  assert.ok(f.nodes().some(node => node.text?.includes('800')));
  f.dispose(); assert.equal(f.stopped(), 1); assert.equal(f.keys.has('keydown'), false);
});

test('no room outside a full-monitor source never invents an overlapping preview or controls; original keyboard finish still works', async () => {
  const f = await fixture(); f.next({ ...initial, hud: { ...initial.hud, preview: null, controls: null } }); await tick();
  assert.ok(f.one('scroll-live-outline')); assert.equal(f.one('scroll-preview'), undefined); assert.equal(f.one('scroll-controls-slot'), undefined);
  f.keys.get('keydown')({ key: 'F8', repeat: false, preventDefault() {}, stopPropagation() {} }); await tick();
  assert.deepEqual(f.calls, [{ id: 'capture-a', action: 'finish' }]); f.dispose();
});

test('fallback platforms keep the original preview/controls, keep is explicit, ending clears all live pixels', async () => {
  const f = await fixture(); f.next({ ...initial, hud: undefined, status: 'lost_overlap' }); await tick();
  assert.equal(f.one('scroll-live-outline'), undefined); assert.ok(f.one('scroll-preview')); assert.ok(f.one('scroll-controls-slot'));
  const keep = f.nodes().find(node => node.props?.['aria-label'] === '保存已拼部分'); assert.ok(keep); keep.props.onClick(); await tick();
  assert.deepEqual(f.calls, [{ id: 'capture-a', action: 'keep' }]);
  f.next(null); await tick(); assert.equal(f.one('scroll-preview'), undefined); assert.equal(f.one('scroll-controls-slot'), undefined); f.dispose();
});

test('compound HUD is transparent/passive outside its controls and preview fits the entire growing image', async () => {
  const rules = new Map(); postcss.parse(await raw('./scroll.css')).walkRules(rule => { const values = rules.get(rule.selector) ?? {}; rule.walkDecls(decl => { values[decl.prop] = decl.value; }); rules.set(rule.selector, values); });
  assert.equal(rules.get('.scroll-compound-window')['pointer-events'], 'none');
  assert.equal(rules.get('.scroll-compound-window .scroll-controls-slot')['pointer-events'], 'auto');
  assert.equal(rules.get('.scroll-live-outline')['pointer-events'], 'none');
  assert.equal(rules.get('.scroll-compound-window .scroll-preview img')['object-fit'], 'contain');
  assert.equal(rules.get('.scroll-compound-window .scroll-preview img')['object-position'], 'center bottom');
  assert.equal(rules.get('.scroll-compound-window .scroll-preview img').height, '100%');
  assert.equal(rules.get('.scroll-compound-window .scroll-preview').overflow, 'hidden');
});
