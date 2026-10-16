// SPDX-License-Identifier: MPL-2.0
// Mount production SettingsDialog, AgentsPanel, ExtensionProviders and the real
// fail-closed capability hook. Only native IPC and unrelated panels are synthetic.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const ts = text => stripTypeScriptTypes(text, { mode: 'transform' });
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const grant = { pluginId: 'mewu.agent', revision: 1, contributionId: 'agent' };
const plugins = { revision: 1, plugins: [{ manifest: { id: 'mewu.agent', contributions: [{ id: 'agent', kind: 'agent.runtime' }] }, revision: 1, state: 'enabled' }] };
const capability = patch => ({ pluginRevision: 1, connectionRevisions: {}, agent: grant, blackboard: null, configuredConnectionIds: [], readyAgentIds: [], boundAgentIds: [], ...patch });
const extension = patch => ({ manifest: { id: 'author.external', name: 'External' }, revision: 1, state: 'enabled', services: [{ id: 'engine', kind: 'agent', title: 'External Agent' }, { id: 'memory', kind: 'memory', title: 'Journal' }], ...patch });
const memoryBinding = patch => ({ agentId: 'agent', kind: 'memory', pluginId: 'author.external', serviceId: 'memory', revision: 1, ...patch });

async function fixture({ initialExtensions = [], initialBindings = [], delayedBindings = false, delayedExtensionRead = false } = {}) {
  const reads = [], extensionListeners = new Set(), bindingListeners = new Set(), pluginListeners = new Set(), domListeners = new Set();
  const bindingRead = deferred(), extensionsRead = deferred();
  let extensionValues = initialExtensions, bindingValues = initialBindings, delayedExtensions = delayedExtensionRead, observedOptional, extensionReads = 0;
  const document = { addEventListener: (name, callback) => domListeners.add(callback), removeEventListener: (name, callback) => domListeners.delete(callback), activeElement: null };
  const context = createContext({ console, document, queueMicrotask, crypto: globalThis.crypto });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [name, value] of Object.entries(values)) this.setExport(name, value); }, { context });
  const core = synthetic(solid), modules = new Map();
  const policy = new SourceTextModule(ts(await raw('./optional-capabilities.ts')), { context }); await policy.link(() => { throw Error('Unexpected policy import'); }); await policy.evaluate();
  const state = new SourceTextModule(ts(await raw('./extension-state.ts')), { context }); await state.link(() => { throw Error('Unexpected state import'); }); await state.evaluate();
  const native = {
    extensionNative: true,
    subscribeExtensions: async callback => { extensionListeners.add(callback); return () => extensionListeners.delete(callback); },
    subscribeExtensionBindings: async callback => { bindingListeners.add(callback); return () => bindingListeners.delete(callback); },
    getExtensions: async () => { extensionReads++; return delayedExtensions ? extensionsRead.promise : extensionValues; },
    getExtensionBindings: async () => delayedBindings ? bindingRead.promise : bindingValues,
    setExtensionBinding: async (agentId, kind, revision, selected) => {
      bindingValues = [memoryBinding({ agentId, kind, revision: (revision ?? 0) + 1, pluginId: selected?.pluginId ?? null, serviceId: selected?.serviceId ?? null })];
      for (const callback of bindingListeners) callback(bindingValues);
      return bindingValues;
    },
  };
  const nativeModule = synthetic(native);
  const pluginBridge = synthetic({
    getOptionalCapabilities: () => { const request = deferred(); reads.push(request); return request.promise; },
    getPlugins: async () => plugins,
    subscribePlugins: async callback => { pluginListeners.add(callback); return () => pluginListeners.delete(callback); },
  });
  const hook = new SourceTextModule(ts(await raw('./optional-feature-state.ts')), { context });
  await hook.link(name => name === 'solid-js' ? core : name === './plugin-bridge' ? pluginBridge : name === './extension-bridge' ? nativeModule : policy); await hook.evaluate();
  const watchedHook = synthetic({ createOptionalFeatureState: (...args) => { observedOptional = hook.namespace.createOptionalFeatureState(...args); return observedOptional; } });
  const universal = new SourceTextModule(await readFile(new URL('../../../node_modules/solid-js/universal/dist/universal.js', import.meta.url), 'utf8'), { context });
  await universal.link(() => core); await universal.evaluate();
  const renderer = universal.namespace.createRenderer({
    createElement: type => ({ type, children: [], props: {}, parent: null, focus() { document.activeElement = this; }, getClientRects: () => [{}], querySelectorAll: () => [] }),
    createTextNode: text => ({ text, children: [], parent: null }), replaceText: (node, text) => { node.text = text; },
    setProperty: (node, name, value) => { node.props[name] = value; },
    insertNode: (parent, node, anchor) => { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); const at = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(at < 0 ? parent.children.length : at, 0, node); node.parent = parent; },
    removeNode: (parent, node) => { parent.children.splice(parent.children.indexOf(node), 1); node.parent = null; },
    isTextNode: node => 'text' in node, getParentNode: node => node.parent, getFirstChild: node => node.children[0], getNextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1],
  });
  const rendererModule = synthetic(renderer), icons = synthetic(Object.fromEntries(['Check', 'ChevronDown', 'ChevronRight', 'LoaderCircle', 'Plus', 'Database', 'Info', 'Link2', 'Puzzle', 'ScanLine', 'Settings2', 'UserRound', 'Wrench', 'X'].map(name => [name, () => renderer.createElement('svg')])));
  const placeholder = name => () => { const node = renderer.createElement(name); return node; };
  const facts = synthetic({ createSettingsInformation: () => ({ info: () => undefined }), AboutSettings: placeholder('about'), DataSettings: placeholder('data') });
  async function component(file) {
    if (modules.has(file)) return modules.get(file);
    const source = await raw(`./components/${file}.tsx`);
    const compiled = ts(transformSync(source, { filename: `${file}.tsx`, babelrc: false, configFile: false, parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]] }).code);
    const module = new SourceTextModule(compiled, { context }); modules.set(file, module);
    await module.link(name => {
      if (name === 'solid-js') return core;
      if (name === 'test-renderer') return rendererModule;
      if (name === 'lucide-solid') return icons;
      if (name === '../optional-feature-state') return watchedHook;
      if (name === '../extension-bridge') return nativeModule;
      if (name === '../extension-state') return state;
      if (name === '../i18n') return synthetic({ t: value => value });
      if (name === '../native-select-escape') return synthetic({ nativeSelectOwnsEscape: () => false });
      if (name === './SettingsFacts') return facts;
      if (name.endsWith('.css')) return synthetic({});
      if (['./AgentsPanel', './ExtensionProviders', './SettingControls'].includes(name)) return component(name.slice(2));
      return synthetic({ default: placeholder(name.slice(2)) });
    });
    await module.evaluate(); return module;
  }
  const dialog = await component('SettingsDialog'), provider = await component('ExtensionProviders');
  const [snapshot, setSnapshot] = solid.createSignal({ activeSceneId: 'scene', scenes: [{ id: 'scene', agentId: 'agent' }], agents: [{ id: 'agent', name: 'My Agent', instructions: 'Original soul', memory: '', memoryEnabled: true, defaultConnectionId: null }], connections: [], memoryStats: [] });
  const root = renderer.createElement('root');
  const dispose = renderer.render(() => dialog.namespace.default({ get snapshot() { return snapshot(); }, initialTab: 'agent', preferences: {}, hotkey: '', onClose() {}, onSaveAgent: async () => {} }), root);
  const all = () => { const list = []; const walk = node => { list.push(node); node.children.forEach(walk); }; walk(root); return list; };
  const text = node => 'text' in node ? node.text : node.children.map(text).join('');
  const find = predicate => all().find(predicate);
  const clickTab = () => { const button = find(node => node.type === 'button' && text(node) === 'Agent'); assert.ok(button, 'Agent subpage is configurable'); button.props.onClick({}); };
  const openAgent = () => { const button = find(node => node.type === 'button' && node.props.class === 'connection-expand'); assert.ok(button); button.props.onClick({}); };
  const editor = () => find(node => node.props?.class === 'connection-editor agent-editor');
  const panel = () => find(node => node.props?.class === 'agents-panel');
  const legacy = () => all().filter(node => ['MemoryProviderSettings', 'MemoryList'].includes(node.type));
  const changed = () => { for (const callback of bindingListeners) callback(bindingValues); };
  const changeExtensions = values => { extensionValues = values; for (const callback of extensionListeners) callback(values); };
  await tick();
  return { root, reads, panel, editor, legacy, clickTab, openAgent, all, find, text, changed, changeExtensions, get optional() { return observedOptional(); }, dispose,
    bindings: values => { bindingValues = values; for (const callback of bindingListeners) callback(values); },
    resolveBindings: value => bindingRead.resolve(value), setSnapshot,
    get listeners() { return { extensions: extensionListeners.size, bindings: bindingListeners.size, plugins: pluginListeners.size, dom: domListeners.size }; },
    get extensionReads() { return extensionReads; }, delayExtensions: () => { delayedExtensions = true; }, resolveExtensions: values => extensionsRead.resolve(values), delayBindings: () => { delayedBindings = true; },
    mountProvider: props => renderer.render(() => provider.namespace.default(props), renderer.createElement('provider-root')) };
}

test('real Settings mount retains expanded Agent and unsaved soul through pending→true, then removes it on actual false', async () => {
  const f = await fixture();
  try {
    assert.equal(f.optional, undefined); assert.equal(f.panel(), undefined);
    f.reads[0].resolve(capability()); await tick(); f.clickTab(); f.openAgent();
    const editor = f.editor(), panel = f.panel(), soul = f.find(node => node.type === 'textarea');
    soul.props.onInput({ currentTarget: { value: 'Unsaved soul draft' } });
    f.changed(); assert.equal(f.optional, undefined); await tick();
    assert.equal(f.panel(), panel); assert.equal(f.editor(), editor); assert.equal(editor.props.hidden, false);
    assert.equal(soul.props.value, 'Unsaved soul draft');
    f.reads.at(-1).resolve(capability()); await tick(); assert.equal(f.panel(), panel); assert.equal(f.editor(), editor); assert.equal(soul.props.value, 'Unsaved soul draft');
    f.changed(); await tick(); assert.equal(f.panel(), panel);
    f.reads.at(-1).resolve(capability({ agent: null })); await tick(); assert.equal(f.panel(), undefined);
    assert.ok(f.find(node => node.type === 'button' && f.text(node) === '连接' && node.props['aria-pressed'] === true));
  } finally { f.dispose(); }
  assert.deepEqual(f.listeners, { extensions: 0, bindings: 0, plugins: 0, dom: 0 });
});

test('an enabled unbound external Agent exposes configuration without a host key while execution stays fail-closed', async () => {
  const f = await fixture({ initialExtensions: [extension()] });
  try {
    f.reads[0].resolve(capability({ agent: null })); await tick();
    assert.equal(f.optional.agent, null); assert.equal(f.optional.readyAgentIds.length, 0); f.clickTab(); f.openAgent(); assert.ok(f.editor());
    f.changed(); await tick(); assert.equal(f.optional, undefined); assert.ok(f.editor());
    f.reads.at(-1).resolve(capability({ agent: null })); await tick(); assert.ok(f.editor());
    f.changeExtensions([extension({ state: 'disabled', revision: 2 })]); await tick(); assert.equal(f.panel(), undefined);
    f.reads.at(-1).resolve(capability({ agent: null })); await tick(); assert.equal(f.panel(), undefined);
    assert.equal(f.extensionReads, 2, 'Settings and its opened provider each read once');
  } finally { f.dispose(); }
});

test('external memory hides legacy local/Hindsight sources without flashing during reads, and clearing restores them', async () => {
  const f = await fixture({ initialExtensions: [extension()], initialBindings: [memoryBinding()], delayedBindings: true });
  try {
    f.reads[0].resolve(capability()); await tick(); f.clickTab(); f.openAgent(); await tick();
    assert.equal(f.legacy().length, 0, 'unknown binding must not flash a local provider');
    f.resolveBindings([memoryBinding()]); await tick(); assert.equal(f.legacy().length, 0);
    const memory = f.find(node => node.type === 'select' && node.props['aria-label'] === '记忆提供方'); assert.ok(memory);
    const editor = f.editor(); f.bindings([memoryBinding({ pluginId: null, serviceId: null, revision: 2 })]); await tick();
    assert.equal(f.legacy().length, 2); assert.equal(f.editor(), editor);
    f.bindings([memoryBinding({ revision: 3 })]); await tick(); assert.equal(f.legacy().length, 0); assert.equal(f.editor(), editor);
    assert.ok(f.find(node => node.type === 'button' && node.props['aria-label'] === '长期记忆'), 'global memory switch remains available');
  } finally { f.dispose(); }
});

test('live external Agent registration wins over a late initial lookup at the same revision', async () => {
  const f = await fixture({ delayedExtensionRead: true });
  try {
    f.reads[0].resolve(capability({ agent: null })); await tick(); assert.equal(f.panel(), undefined);
    f.changeExtensions([extension()]); await tick(); f.clickTab(); assert.ok(f.panel());
    const panel = f.panel(); f.resolveExtensions([]); await tick(); assert.equal(f.panel(), panel);
  } finally { f.dispose(); }
});

test('production provider resets memory presentation on Agent change and retains known selection through same-Agent refreshes', async () => {
  const f = await fixture({ initialBindings: [memoryBinding()], initialExtensions: [] });
  const values = [], [agentId, setAgentId] = solid.createSignal('agent'), [active, setActive] = solid.createSignal(true);
  const stop = f.mountProvider({ get agentId() { return agentId(); }, get active() { return active(); }, saved: true, onMemoryBinding: value => values.push(value) });
  try {
    await tick(); assert.equal(values.at(-1), true);
    f.delayBindings(); setActive(false); setActive(true); await tick(); assert.equal(values.at(-1), true, 'pending refresh keeps a known external provider');
    setAgentId('other-agent'); await tick(); assert.equal(values.at(-1), undefined, 'a different Agent cannot inherit the previous binding');
    f.resolveBindings([]); await tick(); assert.equal(values.at(-1), false);
  } finally { stop(); f.dispose(); }
  assert.deepEqual(f.listeners, { extensions: 0, bindings: 0, plugins: 0, dom: 0 });
});
