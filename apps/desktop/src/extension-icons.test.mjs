// SPDX-License-Identifier: MPL-2.0
// Production Solid components/cache; synthetic IPC and renderer, no native execution.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const tick = () => new Promise(resolve => setImmediate(resolve));
const art = { paths: ['M4 4h16v16H4z', 'M8 8l8 8'], color: '#8b5cf6', filled: false };
const entry = { id: 'independent.tool', name: 'Independent', version: '1.0.0', tags: ['tools'], sha256: 'a'.repeat(64), icon: art };
async function fixture({ initial, entries = [entry], records = [] } = {}) {
  let stored = initial, reads = 0, calls = [], resolveRun;
  const request = new Promise(resolve => { resolveRun = resolve; });
  const context = createContext({ console, queueMicrotask, setTimeout, clearTimeout,
    localStorage: { getItem: () => stored, setItem: (_, value) => { stored = value; } },
    document: { addEventListener() {}, removeEventListener() {} },
  });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [name, value] of Object.entries(values)) this.setExport(name, value); }, { context });
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
  const rendered = synthetic(renderer), api = synthetic({ extensionNative: true,
    getExtensionCatalog: async () => { reads++; return { plugins: entries }; }, getExtensions: async () => records,
    subscribeExtensions: async () => () => {}, invokeExtension: value => { calls.push(value); return request; }, stopExtension: async () => {},
  });
  const modules = new Map();
  async function load(file) {
    if (modules.has(file)) return modules.get(file);
    const source = await raw(file), code = file.endsWith('.tsx') ? transformSync(source, { filename: file, babelrc: false, configFile: false,
      parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'test-renderer' }]] }).code : source;
    const module = new SourceTextModule(stripTypeScriptTypes(code, { mode: 'transform' }), { context }); modules.set(file, module);
    await module.link(async name => {
      if (name === 'solid-js') return core;
      if (name === 'test-renderer') return rendered;
      if (name.endsWith('extension-bridge')) return api;
      if (name.endsWith('i18n')) return synthetic({ t: value => value });
      if (name === 'lucide-solid') {
        const names = [...source.matchAll(/import\s*\{([^}]+)\}\s*from 'lucide-solid'/g)].flatMap(match => match[1].split(',').map(name => name.trim()));
        return synthetic(Object.fromEntries(names.map(name => [name, () => { const node = renderer.createElement('svg'); node.props.glyph = name; return node; }])));
      }
      if (name.endsWith('.css')) return synthetic({});
      const target = new URL(name + (name.endsWith('PluginIcon') ? '.tsx' : '.ts'), new URL(file, import.meta.url));
      return load('./' + target.pathname.split('/src/')[1]);
    });
    await module.evaluate(); return module;
  }
  const cache = (await load('./extension-icons.ts')).namespace;
  const icon = (await load('./components/PluginIcon.tsx')).namespace.default;
  const root = renderer.createElement('root'), disposals = [];
  const mountIcon = props => disposals.push(renderer.render(() => icon(props), root));
  const all = () => { const result = []; const visit = node => { result.push(node); node.children.forEach(visit); }; visit(root); return result; };
  return { cache, mountIcon, root, all, calls, resolveRun, get reads() { return reads; }, get stored() { return stored; },
    mountCommands: async props => { const component = (await load('./components/ExtensionSpaceCommands.tsx')).namespace.default; disposals.push(renderer.render(() => component(props), root)); await tick(); },
    dispose: () => disposals.forEach(dispose => dispose()),
  };
}

test('market, installed package and toolbar render identical safe paths, with exact release identity', async () => {
  const f = await fixture();
  try {
    f.mountIcon({ plugin: entry });
    f.mountIcon({ plugin: { ...entry, icon: undefined }, sha256: entry.sha256 });
    f.mountIcon({ plugin: { ...entry, icon: undefined }, source: { type: 'github', entry } });
    await tick(); await tick();
    assert.equal(f.reads, 1, 'catalog requests shared across icons');
    assert.deepEqual(f.all().filter(node => node.type === 'path').map(node => node.props.d), [...art.paths, ...art.paths, ...art.paths]);
    assert.equal(f.cache.cachedExtensionIcon(entry.id, '2.0.0', entry.sha256), undefined);
    assert.equal(f.cache.cachedExtensionIcon(entry.id, entry.version, 'b'.repeat(64)), undefined);
    assert.ok(f.cache.cachedExtensionIcon(entry.id, entry.version, entry.sha256));
    assert.equal(f.all().some(node => ['img', 'script', 'foreignObject'].includes(node.type)), false);
  } finally { f.dispose(); }
});

test('offline cache is bounded and malformed artwork never adds markup or external resources', async () => {
  for (const initial of ['broken json', 'x'.repeat(2 * 1024 * 1024 + 1), JSON.stringify(Array(257).fill(['x', art]))]) {
    const f = await fixture({ initial, entries: [] });
    try { assert.equal(f.cache.cachedExtensionIcon(entry.id, entry.version, entry.sha256), undefined); } finally { f.dispose(); }
  }
  const f = await fixture({ entries: [], initial: JSON.stringify([[JSON.stringify([entry.id, entry.version, entry.sha256]), art]]) });
  try {
    assert.ok(f.cache.cachedExtensionIcon(entry.id, entry.version, entry.sha256));
    for (const icon of [{ paths: [] }, { paths: ['M'.repeat(4097)] }, { paths: ['M0 0'], color: 'url(https://example.com)' }, { paths: ['M0 0'], filled: 'true' }, '<svg onload=alert(1)>']) assert.equal(f.cache.displayIcon(icon), undefined);
    f.mountIcon({ plugin: { ...entry, icon: { ...art, onload: 'alert(1)', html: '<script>' } } });
    assert.equal(f.all().some(node => node.props.onload || node.props.innerHTML || node.props.src), false);
  } finally { f.dispose(); }
});

test('individual lower-left command buttons show their own artwork and preserve Stop until completion', async () => {
  const records = [entry, { ...entry, id: 'independent.memory', icon: { ...art, paths: ['M12 2v20'] } }].map((manifest, index) => ({ manifest, sha256: entry.sha256, revision: 1, state: 'enabled', source: { type: 'local', path: 'test.zip' }, services: [{ kind: 'command', id: 'open', title: `Plugin ${index + 1}`, metadata: { placement: 'space-toolbar' } }] }));
  const f = await fixture({ records });
  try {
    await f.mountCommands({ sceneId: 'scene', disabled: false, onError: message => { throw Error(message); } });
    const buttons = f.all().filter(node => node.type === 'button');
    assert.deepEqual(buttons.map(node => node.props['data-plugin-id']).sort(), records.map(record => record.manifest.id).sort());
    for (const button of buttons) {
      const paths = []; const visit = node => { if (node.type === 'path') paths.push(node.props.d); node.children.forEach(visit); }; visit(button);
      assert.deepEqual(paths, records.find(record => record.manifest.id === button.props['data-plugin-id']).manifest.icon.paths);
    }
    buttons[1].props.onClick(); await tick();
    assert.equal(f.calls.length, 1); assert.equal(f.calls[0].pluginId, buttons[1].props['data-plugin-id']);
    assert.equal(buttons[1].props['aria-label'], '停止插件执行'); assert.equal(buttons[0].props.disabled, true);
    buttons[1].props.onClick(); await tick(); assert.equal(buttons[0].props.disabled, true, 'late reply still owns invocation slot');
    f.resolveRun({ content: 'late canceled' }); await tick();
    assert.equal(f.all().some(node => node.props?.role === 'status'), false);
    assert.equal(buttons[0].props.disabled, false);
  } finally { f.dispose(); }
});
