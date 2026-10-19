// SPDX-License-Identifier: MPL-2.0
// Production Solid component + production lease/portal controllers. Native IPC
// is synthetic: these tests do not register shortcuts or prove an OS hotkey.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule, createContext } from 'node:vm';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const tick = () => new Promise(resolve => setImmediate(resolve));
const chord = { code: 'KeyS', ctrl: false, shift: true, alt: true };
const native = extra => ({ revision: 7, sequence: 9, configured: chord, active: chord, status: 'active', editable: true, ...extra });
const compiled = stripTypeScriptTypes(transformSync(await raw('./components/CaptureShortcutField.tsx'), {
  filename: 'CaptureShortcutField.tsx', configFile: false, babelrc: false,
  parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]],
}).code, { mode: 'transform' });

async function fixture(initial) {
  const calls = [], listeners = new Map(), docListeners = new Map(); let hooks, disposed, serial = 0;
  const document = { visibilityState: 'visible', activeElement: null, hasFocus: () => true,
    addEventListener: (name, fn) => docListeners.set(name, fn), removeEventListener: name => docListeners.delete(name) };
  const window = { addEventListener: (name, fn) => listeners.set(name, fn), removeEventListener: name => listeners.delete(name) };
  const context = createContext({ document, window, console, Promise, crypto: { randomUUID: () => `lease-${++serial}` } });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const core = synthetic(solid);
  const load = async path => { const module = new SourceTextModule(stripTypeScriptTypes(await raw(path), { mode: 'transform' }), { context }); await module.link(() => { throw Error('Unexpected runtime controller import'); }); await module.evaluate(); return module; };
  const editor = await load('./capture-shortcut-editor.ts'), portal = await load('./capture-shortcut-portal.ts');
  const universal = new SourceTextModule(await raw('../../../node_modules/solid-js/universal/dist/universal.js'), { context });
  await universal.link(() => core); await universal.evaluate();
  const renderer = universal.namespace.createRenderer({
    createElement: type => ({ type, children: [], props: {}, parent: null, blur() { document.activeElement = null; this.props.onBlur?.(); } }),
    createTextNode: text => ({ text, children: [], parent: null }), replaceText: (node, text) => { node.text = text; },
    setProperty: (node, key, value) => { node.props[key] = value; },
    insertNode: (parent, node, anchor) => { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); const index = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(index < 0 ? parent.children.length : index, 0, node); node.parent = parent; },
    removeNode: (parent, node) => { parent.children.splice(parent.children.indexOf(node), 1); node.parent = null; },
    isTextNode: node => 'text' in node, getParentNode: node => node.parent, getFirstChild: node => node.children[0], getNextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1],
  });
  const bridge = {
    subscribeCaptureShortcut: async value => { hooks = value; return () => calls.push(['stop']); }, getCaptureShortcut: async () => initial,
    beginCaptureShortcutEdit: async id => { calls.push(['begin', id]); return { leaseId: id, expiresAt: Date.now() + 30000 }; },
    endCaptureShortcutEdit: async id => { calls.push(['end', id]); },
    setCaptureShortcut: async (revision, shortcut) => { calls.push(['save', revision, shortcut]); return native({ configured: shortcut, active: shortcut, revision: revision + 1, sequence: 10 }); },
    configureCaptureShortcutPortal: async (revision, id) => { calls.push(['configure', revision, id]); return { ...initial, status: 'active', actualTriggerDescription: 'Alt + Shift + S' }; },
    cancelCaptureShortcutPortal: async id => { calls.push(['cancel-portal', id]); },
    openCaptureShortcutSystemSettings: async () => { calls.push(['system-settings']); },
  };
  const module = new SourceTextModule(compiled, { context });
  await module.link(name => {
    if (name === 'solid-js') return core;
    if (name === 'test-renderer') return synthetic(renderer);
    if (name === '../i18n') return synthetic({ t: value => value });
    if (name === '../shortcut-bridge') return synthetic(bridge);
    if (name === '../capture-shortcut-editor') return editor;
    if (name === '../capture-shortcut-portal') return portal;
    if (name === 'lucide-solid') return synthetic({ Check: () => null, LoaderCircle: () => null, RotateCcw: () => null, X: () => null });
    if (name.endsWith('.css')) return synthetic({});
    throw Error(`Unexpected import ${name}`);
  }); await module.evaluate();
  const root = { children: [] }; let setVisible;
  solid.createRoot(dispose => { disposed = dispose; const [visible, update] = solid.createSignal(true); setVisible = update; renderer.insert(root, () => module.namespace.default({ get visible() { return visible(); } })); });
  await tick();
  const nodes = () => { const result = []; const walk = node => { result.push(node); node.children?.forEach(walk); }; walk(root); return result; };
  const text = node => node.text ?? node.children.map(text).join('');
  const button = name => nodes().find(node => node.type === 'button' && (node.props['aria-label'] === name || text(node) === name));
  return { calls, nodes, button, document, listeners, update: value => hooks.state(value), visible: setVisible, dispose: () => disposed() };
}

test('native backend uses the same editable recorder and flushes its exact lease before one CAS save', async () => {
  const f = await fixture(native());
  const input = f.nodes().find(node => node.type === 'input'); assert.ok(input); assert.equal(Boolean(input.props.disabled), false);
  f.document.activeElement = input; input.props.onFocus(); await tick(); assert.equal(f.calls[0][0], 'begin');
  const event = { key: 's', ctrlKey: true, shiftKey: true, altKey: false, metaKey: false, repeat: false, isComposing: false, keyCode: 0, preventDefault() {}, stopPropagation() {} };
  assert.equal(typeof input.props['on:keydown'], 'function'); input.props['on:keydown'](event);
  f.button('保存截图快捷键').props.onClick(); await tick();
  assert.deepEqual(JSON.parse(JSON.stringify(f.calls.slice(0, 3))), [['begin', 'lease-1'], ['end', 'lease-1'], ['save', 7, { code: 'KeyS', ctrl: true, shift: true, alt: false }]]);
  f.dispose();
});

test('Wayland v1 setup and active modify use portal or system settings and never arm the native key recorder', async () => {
  const f = await fixture(native({ backend: 'wayland_portal', status: 'needs_setup', active: null, configurationPending: false, systemSettingsAvailable: true }));
  assert.equal(f.nodes().some(node => node.type === 'input'), false);
  const setup = f.button('设置'); assert.ok(setup); setup.props.onClick(); await tick();
  assert.equal(f.calls[0][0], 'configure'); assert.equal(f.calls[0][1], 7);
  const modify = f.button('修改'); assert.ok(modify); modify.props.onClick(); await tick();
  assert.equal(f.calls.at(-1)[0], 'system-settings');
  assert.equal(f.calls.some(call => call[0] === 'begin' || call[0] === 'save'), false); f.dispose();
});

test('Wayland with no system editor never presents a native input or a nonfunctional modify button', async () => {
  const f = await fixture(native({ backend: 'wayland_portal', actualTriggerDescription: 'System key', systemSettingsAvailable: false }));
  assert.equal(f.nodes().some(node => node.type === 'input'), false); assert.equal(f.button('修改'), undefined);
  assert.equal(f.calls.length, 0); f.dispose();
});

test('hiding settings or changing backend revokes the native input lease without saving a draft', async () => {
  for (const action of ['hide', 'portal', 'blur']) {
    const f = await fixture(native()); const input = f.nodes().find(node => node.type === 'input');
    f.document.activeElement = input; input.props.onFocus(); await tick();
    if (action === 'hide') f.visible(false);
    else if (action === 'portal') f.update(native({ sequence: 10, backend: 'wayland_portal' }));
    else f.listeners.get('blur')();
    await tick(); assert.deepEqual(f.calls.filter(call => call[0] !== 'stop'), [['begin', 'lease-1'], ['end', 'lease-1']]); f.dispose();
  }
});
