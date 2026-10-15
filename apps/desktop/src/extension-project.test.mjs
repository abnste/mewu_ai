// SPDX-License-Identifier: MPL-2.0
// Production controller/bridge and actual Solid JSX, synthetic IPC only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from 'solid-js/dist/solid.js';
const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const ts = text => stripTypeScriptTypes(text, { mode: 'transform' });
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const entry = (patch = {}) => ({ id: 'own.plugin', version: '1.0.0', sha256: 'a'.repeat(64), repository: 'own/package', releaseTag: 'v1.0.0', assetName: 'plugin.zip', ...patch });
const project = (value = entry(), count = 3) => ({ id: value.id, version: value.version, packageSha256: value.sha256, sourceRepository: 'own/source', sourceCommit: 'b'.repeat(40), screenshots: Array.from({ length: count }, (_, index) => ({ path: `screenshots/${index}.png`, sha256: 'c'.repeat(64), size: 100, width: 16, height: 16, caption: `Own test ${index}` })) });
const png = 'data:image/png;base64,iVBORw0KGgo=';
async function stateModule(context) {
  const module = new SourceTextModule(ts(await raw('./extension-project-state.ts')), context ? { context } : undefined);
  const core = new SyntheticModule(Object.keys(solid), function () { for (const [key, value] of Object.entries(solid)) this.setExport(key, value); }, context ? { context } : undefined);
  await module.link(() => core); await module.evaluate(); return module;
}
const state = (await stateModule()).namespace;

test('project metadata is detail-only, same-source snapshots do not refetch, and old indexes stay usable', async () => {
  let metadata = 0, images = 0;
  const value = state.createExtensionProjectState({ getExtensionProject: async () => { metadata++; return null; }, getExtensionProjectImage: async () => { images++; return png; } });
  value.setEntry(); assert.equal(metadata, 0);
  value.setEntry(entry()); await tick(); assert.equal(metadata, 1); assert.equal(value.project(), null);
  value.setEntry(structuredClone(entry())); await tick(); assert.equal(metadata, 1); assert.equal(images, 0);
  value.setEntry(); value.dispose(); assert.equal(value.project(), undefined);
  assert.equal(state.projectRepositoryUrl('own/source'), 'https://github.com/own/source');
  for (const source of ['https://evil.example', 'own/source/other', '../source', 'own/source?key=a']) assert.equal(state.projectRepositoryUrl(source), undefined);
});

test('images are lazy, single-flight, prioritized and limited to two actual pending IPCs', async () => {
  const calls = [], requests = [];
  const value = state.createExtensionProjectState({ getExtensionProject: async () => project(), getExtensionProjectImage: async (_entry, index, commit, sha) => { assert.equal(commit, 'b'.repeat(40)); assert.equal(sha, 'c'.repeat(64)); calls.push(index); const pending = deferred(); requests.push(pending); return pending.promise; } });
  value.setEntry(entry()); await tick(); assert.deepEqual(calls, [0]);
  value.loadImage(1); value.loadImage(2); value.loadImage(2); value.select(2);
  assert.deepEqual(calls, [0, 1]);
  requests[0].resolve(png); await tick(); assert.deepEqual(calls, [0, 1, 2]); assert.equal(value.images()[0], png);
  requests[1].resolve(png); requests[2].resolve(png); await tick();
  value.loadImage(2); assert.equal(calls.length, 3); assert.equal(value.selected(), 2);
  value.setEntry(); assert.deepEqual(Object.keys(value.images()), []); value.dispose();
});

test('closing and reopening owners shares the actual in-flight image budget', async () => {
  const requests = [], calls = []; let active = 0, maximum = 0;
  const api = { getExtensionProject: async item => project(item), getExtensionProjectImage: async (item, index) => {
    const pending = deferred(); requests.push(pending); calls.push([item.id, index]); active++; maximum = Math.max(maximum, active);
    try { return await pending.promise; } finally { active--; }
  } };
  const first = state.createExtensionProjectState(api);
  first.setEntry(entry()); await tick(); first.loadImage(1); first.loadImage(2);
  assert.equal(calls.length, 2); first.dispose();
  const second = state.createExtensionProjectState(api);
  second.setEntry(entry({ id: 'next.plugin' })); await tick();
  assert.equal(calls.length, 2); assert.equal(second.pendingImages().has(0), true);
  requests[0].resolve(png); await tick(); assert.deepEqual(calls[2], ['next.plugin', 0]);
  requests[1].resolve(png); await tick(); assert.equal(second.pendingImages().has(0), true);
  assert.deepEqual(Object.keys(first.images()), []); assert.equal(calls.length, 3);
  requests[2].resolve(png); await tick(); assert.equal(second.images()[0], png);
  assert.equal(maximum, 2); assert.equal(active, 0); second.dispose();
});

test('closed and replaced projects reject late metadata/images without pretending requests have settled', async () => {
  const firstProject = deferred(), image = deferred(); let reads = 0, active = 0, maximum = 0;
  const value = state.createExtensionProjectState({ getExtensionProject: async item => ++reads === 1 ? firstProject.promise : project(item),
    getExtensionProjectImage: async () => { active++; maximum = Math.max(maximum, active); try { return await image.promise; } finally { active--; } } });
  value.setEntry(entry()); value.setEntry(entry({ version: '2.0.0' })); await tick();
  firstProject.resolve(project()); await tick(); assert.equal(value.project().version, '2.0.0');
  value.setEntry(); image.resolve(png); await tick(); assert.equal(value.project(), undefined); assert.deepEqual(Object.keys(value.images()), []); assert.ok(maximum <= 2);
  value.dispose();
  const late = deferred(); let imageReads = 0;
  const disposed = state.createExtensionProjectState({ getExtensionProject: () => late.promise, getExtensionProjectImage: async () => { imageReads++; return png; } });
  disposed.setEntry(entry()); disposed.dispose(); late.resolve(project()); await tick(); assert.equal(imageReads, 0);
});

test('source changes and bad data URLs fail closed while explicit retry is bounded', async () => {
  let reads = 0;
  const value = state.createExtensionProjectState({ getExtensionProject: async item => project(item), getExtensionProjectImage: async () => ++reads === 1 ? 'https://unverified.example/image.png' : png });
  value.setEntry(entry()); await tick(); assert.equal(value.failedImages().has(0), true); assert.equal(value.images()[0], undefined); assert.equal(reads, 1);
  value.retry(); await tick(); assert.equal(reads, 2); assert.equal(value.images()[0], png); value.dispose();
  let images = 0;
  const mismatch = state.createExtensionProjectState({ getExtensionProject: async () => ({ ...project(), packageSha256: 'd'.repeat(64) }), getExtensionProjectImage: async () => { images++; return png; } });
  mismatch.setEntry(entry()); await tick(); assert.equal(mismatch.projectError(), true); assert.equal(images, 0); mismatch.dispose();
});

test('explicit image retry refreshes metadata rather than replaying an expired snapshot', async () => {
  const calls = []; let revision = 0;
  const value = state.createExtensionProjectState({ getExtensionProject: async () => ({ ...project(), sourceCommit: (revision++ ? 'd' : 'b').repeat(40) }),
    getExtensionProjectImage: async (_entry, index, commit, sha) => {
      calls.push({ index, commit, sha }); if (commit === 'b'.repeat(40)) throw new Error('Snapshot changed'); return png;
    } });
  value.setEntry(entry()); await tick(); assert.equal(value.failedImages().has(0), true);
  value.retry(); await tick(); assert.equal(revision, 2); assert.equal(value.project().sourceCommit, 'd'.repeat(40));
  assert.deepEqual(calls.map(value => value.commit), ['b'.repeat(40), 'd'.repeat(40)]); assert.equal(value.images()[0], png); value.dispose();
});

test('native project bridge uses entry/index envelopes; ordinary browser never invents project data', async () => {
  for (const native of [true, false]) {
    const calls = [];
    const module = new SourceTextModule(ts(await raw('./extension-bridge.ts')));
    await module.link(name => new SyntheticModule(name.endsWith('/core') ? ['isTauri', 'invoke'] : ['listen'], function () {
      if (name.endsWith('/core')) { this.setExport('isTauri', () => native); this.setExport('invoke', async (command, params) => { calls.push({ command, params }); return null; }); }
      else this.setExport('listen', async () => () => {});
    })); await module.evaluate();
    if (native) {
      await module.namespace.getExtensionProject(entry()); await module.namespace.getExtensionProjectImage(entry(), 2, 'b'.repeat(40), 'c'.repeat(64));
      assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{ command: 'get_extension_project', params: { entry: entry() } }, { command: 'get_extension_project_image', params: { entry: entry(), index: 2, expectedSourceCommit: 'b'.repeat(40), expectedImageSha256: 'c'.repeat(64) } }]);
    } else {
      await assert.rejects(module.namespace.getExtensionProject(entry()), /桌面版/);
      await assert.rejects(module.namespace.getExtensionProjectImage(entry(), 0, 'b'.repeat(40), 'c'.repeat(64)), /桌面版/); assert.equal(calls.length, 0);
    }
  }
});

test('actual Solid gallery lazily observes thumbnails, opens real returned URLs and owns Escape/disposal', async () => {
  const observers = [], calls = [], links = []; let document, metadata = project();
  class Element {
    constructor(type) { this.type = type; this.children = []; this.props = {}; this.parent = null; this.isConnected = true; }
    focus() { document.activeElement = this; }
    contains(target) { return target === this || this.children.some(child => child.contains?.(target)); }
    querySelectorAll() { const nodes = []; const walk = node => { if (node.type === 'button' && !node.props.disabled) nodes.push(node); node.children?.forEach(walk); }; walk(this); return nodes; }
  }
  document = new EventTarget(); document.activeElement = new Element('original'); document.body = new Element('body');
  const context = createContext({ console, document, HTMLElement: Element, queueMicrotask, IntersectionObserver: class { constructor(callback) { this.callback = callback; observers.push(this); } observe() {} disconnect() { this.disconnected = true; } } });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const core = synthetic(solid);
  const universal = new SourceTextModule(await readFile(new URL(import.meta.resolve('solid-js/universal')), 'utf8'), { context });
  await universal.link(() => core); await universal.evaluate();
  const renderer = universal.namespace.createRenderer({ createElement: type => new Element(type), createTextNode: text => ({ text, children: [], parent: null }), replaceText: (node, text) => node.text = text,
    setProperty: (node, name, value) => node.props[name] = value,
    insertNode(parent, node, anchor) { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); const at = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(at < 0 ? parent.children.length : at, 0, node); node.parent = parent; },
    removeNode(parent, node) { parent.children.splice(parent.children.indexOf(node), 1); node.parent = null; },
    isTextNode: node => 'text' in node, getParentNode: node => node.parent, getFirstChild: node => node.children[0], getNextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] });
  const controller = await stateModule(context);
  const compiled = ts(transformSync(await raw('./components/ExtensionProjectGallery.tsx'), { filename: 'ExtensionProjectGallery.tsx', babelrc: false, configFile: false, parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'own-renderer' }]] }).code);
  const component = new SourceTextModule(compiled, { context });
  await component.link(name => name === 'solid-js' ? core : name === 'own-renderer' ? synthetic(renderer)
    : name === 'solid-js/web' ? synthetic({ Portal: props => props.children })
      : name === '../extension-project-state' ? controller
        : name === '../extension-bridge' ? synthetic({ extensionNative: true, getExtensionProject: async () => metadata, getExtensionProjectImage: async (_entry, index) => { calls.push(index); return png; } })
          : name === '../reply-content-bridge' ? synthetic({ nativeReplyLinks: () => true, openReplyLink: async url => links.push(url) })
            : name === '../i18n' ? synthetic({ t: value => value })
              : synthetic(Object.fromEntries(['ArrowLeft', 'ArrowRight', 'ExternalLink', 'Image', 'LoaderCircle', 'X'].map(key => [key, () => renderer.createElement('svg')]))));
  await component.evaluate(); const root = renderer.createElement('root');
  const dispose = renderer.render(() => component.namespace.default({ active: true, entry: entry(), name: 'Own test plugin' }), root);
  const all = (host = root) => { const nodes = []; const walk = node => { nodes.push(node); node.children?.forEach(walk); }; walk(host); return nodes; };
  await tick(); assert.deepEqual(calls, [0]); assert.equal(observers.length, 3);
  observers[1].callback([{ isIntersecting: true }]); await tick(); assert.deepEqual(calls, [0, 1]);
  const hero = all().find(node => node.props?.class === 'plugin-project-main'); assert.ok(hero);
  hero.props.onClick(); await tick();
  const viewer = all().find(node => node.props?.class === 'plugin-project-viewer'); assert.ok(viewer);
  assert.ok(all().some(node => node.type === 'img' && node.props.src === png));
  const key = value => { const event = new Event('keydown', { cancelable: true }); Object.defineProperty(event, 'key', { value }); document.dispatchEvent(event); return event; };
  const next = all().find(node => node.type === 'button' && node.props['aria-label'] === '下一张');
  next.props.onClick(); await tick(); next.props.onClick(); await tick(); assert.equal(next.props.disabled, true);
  document.activeElement = document.body;
  const left = key('ArrowLeft'); await tick(); assert.equal(left.defaultPrevented, true); assert.equal(next.props.disabled, false); assert.notEqual(document.activeElement, document.body);
  document.activeElement = document.body;
  const escape = key('Escape'); await tick();
  assert.equal(escape.defaultPrevented, true); assert.equal(all().some(node => node.props?.class === 'plugin-project-viewer'), false);
  const afterClose = key('ArrowLeft'); assert.equal(afterClose.defaultPrevented, false);
  const sourceUrl = `https://github.com/own/source/tree/${'b'.repeat(40)}`;
  const source = all().find(node => node.type === 'a' && node.props.href === sourceUrl); assert.ok(source); source.props.onClick({ preventDefault() {} }); await tick(); assert.deepEqual(links, [sourceUrl]);
  dispose(); assert.ok(observers.every(observer => observer.disconnected));
  metadata = null;
  const oldRoot = renderer.createElement('root');
  const absent = renderer.render(() => component.namespace.default({ active: true, entry: entry(), name: 'Old catalog' }), oldRoot);
  await tick();
  assert.equal(all(oldRoot).filter(node => node.type === 'a').length, 1);
  assert.equal(all(oldRoot).find(node => node.type === 'a').props.href, 'https://github.com/own/package/releases/tag/v1.0.0');
  assert.equal(all(oldRoot).some(node => node.props?.class === 'plugin-project-main'), false);
  absent();
});
