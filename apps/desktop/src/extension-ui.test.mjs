// SPDX-License-Identifier: MPL-2.0
// Actual Solid logic and native wire adapters with isolated synthetic IPC only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
import * as reactive from '../../../node_modules/solid-js/dist/solid.js';

const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const plain = source => stripTypeScriptTypes(source, { mode: 'transform' });
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const normalize = value => JSON.parse(JSON.stringify(value));
const tick = () => new Promise(resolve => setImmediate(resolve));
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
const stateModule = new SourceTextModule(plain(await raw('./extension-state.ts'))); await stateModule.link(() => { throw new Error('Unexpected value import'); }); await stateModule.evaluate();
const state = stateModule.namespace;
const manifest = { schemaVersion: 2, hostApi: 2, runtime: 'javascript', id: 'independent.tool', version: '1.0.0', name: 'Independent', description: 'Synthetic', license: 'MPL-2.0', entry: 'index.js', permissions: [], tags: ['tools'], dependencies: {} };
const command = { kind: 'command', id: 'operate', title: '区域操作', metadata: { placement: 'selection-toolbar' } };
const record = (patch = {}) => ({ manifest, revision: 1, state: 'enabled', source: { type: 'local', path: 'synthetic.zip' }, sha256: 'a'.repeat(64), hasRollback: false, services: [command], ...patch });
const entry = { ...manifest, repository: 'independent/tool', releaseTag: 'v1.0.0', assetName: 'tool.zip', size: 100, sha256: 'a'.repeat(64), platforms: ['any'] };
async function controller(filename, api, expose, props) {
  const source = await raw(filename), ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  const component = ast.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration;
  const returning = component.body.body.find(node => node.type === 'ReturnStatement');
  // Keep production hooks/callbacks verbatim, replace only its final JSX render for inspection.
  const inspected = source.slice(0, returning.start) + `return {${expose.join(',')}};` + source.slice(returning.end);
  const module = new SourceTextModule(plain(inspected));
  const importedNames = name => ast.program.body.filter(node => node.type === 'ImportDeclaration' && node.source.value === name)
    .flatMap(node => node.specifiers.map(specifier => specifier.type === 'ImportDefaultSpecifier' ? 'default' : specifier.imported?.name).filter(Boolean));
  await module.link(name => name === 'solid-js' ? synthetic(reactive)
    : name.endsWith('/extension-bridge') ? synthetic(api)
      : name.endsWith('/extension-state') ? synthetic(state)
        : name.endsWith('/i18n') ? synthetic({ t: text => text })
          : name.endsWith('/plugin-categories') ? synthetic({ pluginTagLabels: { tools: '工具与能力' } }) : synthetic(Object.fromEntries(importedNames(name).map(key => [key, () => {}]))));
  await module.evaluate();
  let value, dispose;
  reactive.createRoot(cleanup => { dispose = cleanup; value = module.namespace.default(props); });
  return { value, dispose };
}

test('dynamic registrations are discovered by kind/placement, not plugin identity; disabled versions disappear', () => {
  const values = [record(), record({ manifest: { ...manifest, id: 'other.agent' }, services: [{ kind: 'agent', id: 'engine', title: 'Agent' }] }),
    record({ manifest: { ...manifest, id: 'disabled' }, state: 'disabled' }), record({ manifest: { ...manifest, id: 'no.placement' }, services: [{ ...command, metadata: [] }] })];
  assert.deepEqual(Array.from(state.selectionExtensionServices(values), choice => choice.record.manifest.id), ['independent.tool']);
  assert.deepEqual(Array.from(state.extensionServices(values, 'agent'), choice => choice.service.id), ['engine']);
  const latest = state.mergeExtensionRecords([record({ revision: 5, state: 'removed' })], [record({ revision: 4 })]); assert.equal(latest[0].state, 'removed');
});

test('binding tombstones survive older get acknowledgments and explicitly represent default provider', () => {
  const latest = { agentId: 'agent', kind: 'memory', pluginId: null, serviceId: null, revision: 5 };
  const values = state.mergeExtensionBindings([latest], [{ ...latest, revision: 4, pluginId: 'old', serviceId: 'memory' }]);
  assert.deepEqual(normalize(values), [latest]);
});

test('ordinary browser bridge refuses native actions instead of inventing a catalog or install success', async () => {
  let calls = 0, listens = 0;
  const module = new SourceTextModule(plain(await raw('./extension-bridge.ts')));
  await module.link(name => name.endsWith('/core') ? synthetic({ isTauri: () => false, invoke: async () => { calls++; } }) : synthetic({ listen: async () => { listens++; } })); await module.evaluate();
  for (const method of ['getExtensions', 'getExtensionCatalog', 'prepareExtension', 'getExtensionBindings']) await assert.rejects(module.namespace[method](), /桌面版/);
  assert.equal(calls, 0); assert.equal(listens, 0);
});

test('native bridge sends exact typed invocation, revision, stop and binding envelopes', async () => {
  const calls = [], events = [], module = new SourceTextModule(plain(await raw('./extension-bridge.ts')));
  await module.link(name => name.endsWith('/core') ? synthetic({ isTauri: () => true, invoke: async (name, args) => { calls.push({ name, args }); return []; } })
    : synthetic({ listen: async (name, callback) => { events.push({ name, callback }); return () => {}; } })); await module.evaluate();
  const invocation = { pluginId: manifest.id, revision: 4, serviceId: 'operate', input: { task: 'task' }, sceneId: 'scene', regionId: 'region' };
  await module.namespace.invokeExtension(invocation); await module.namespace.controlExtension(record({ revision: 4 }), 'disable');
  await module.namespace.setExtensionBinding('agent', 'memory', 7, { pluginId: manifest.id, serviceId: 'memory' });
  await module.namespace.setExtensionBinding('agent', 'memory', 8); await module.namespace.stopExtension(manifest.id);
  assert.deepEqual(normalize(calls), [
    { name: 'invoke_extension', args: { invocation } },
    { name: 'control_extension', args: { pluginId: manifest.id, revision: 4, action: 'disable' } },
    { name: 'set_extension_binding', args: { binding: { agentId: 'agent', kind: 'memory', pluginId: manifest.id, serviceId: 'memory', expectedRevision: 7 } } },
    { name: 'set_extension_binding', args: { binding: { agentId: 'agent', kind: 'memory', pluginId: null, serviceId: null, expectedRevision: 8 } } },
    { name: 'stop_extension', args: { pluginId: manifest.id } },
  ]);
  await module.namespace.subscribeExtensionBindings(() => {}); assert.equal(events[0].name, 'extension-bindings');
});

test('market preserves same-revision registration events over a late initial query and fences prepared package navigation', async () => {
  const initial = deferred(), preparing = deferred(); let receive, subscriptions = 0, stops = 0;
  const api = { extensionNative: true, subscribeExtensions: async callback => { subscriptions++; receive = callback; return () => stops++; },
    getExtensions: () => initial.promise, getExtensionCatalog: async () => ({ schemaVersion: 2, plugins: [entry] }), prepareExtension: () => preparing.promise };
  const props = { active: true, pane: 'discover', query: '', filter: '', addRequest: 0, refreshRequest: 0, onDetail() {} };
  const { value, dispose } = await controller('./components/CommunityExtensions.tsx', api, ['records', 'catalog', 'detail', 'proposal', 'refresh', 'prepare', 'back', 'error'], props);
  await tick(); receive([record()]); initial.resolve([record({ services: [] })]); await tick();
  assert.equal(value.records()[0].services.length, 1); assert.equal(value.catalog().length, 1);
  const preparingTask = value.prepare(entry); await tick(); value.back(); preparing.resolve({ id: 'proposal', manifest, source: record().source, sha256: 'a'.repeat(64) }); await preparingTask;
  assert.equal(value.proposal(), undefined); assert.equal(value.detail(), undefined);
  await Promise.all([value.refresh(), value.refresh()]); assert.equal(subscriptions, 1); dispose(); assert.equal(stops, 1);
});

test('failed install retains the reviewed proposal and never reports installed', async () => {
  const proposed = { id: 'proposal', manifest, source: record().source, sha256: 'a'.repeat(64) };
  const api = { extensionNative: true, subscribeExtensions: async () => () => {}, getExtensions: async () => [], getExtensionCatalog: async () => ({ schemaVersion: 2, plugins: [entry] }),
    prepareExtension: async () => proposed, installExtension: async () => { throw new Error('activation failed'); } };
  const { value, dispose } = await controller('./components/CommunityExtensions.tsx', api, ['records', 'proposal', 'prepare', 'install', 'error'], { active: true, pane: 'discover', query: '', filter: '', addRequest: 0, refreshRequest: 0, onDetail() {} });
  await tick(); await value.prepare(entry); await value.install();
  assert.equal(value.proposal().id, 'proposal'); assert.equal(value.records().length, 0); assert.equal(value.error(), 'activation failed'); dispose();
});

test('region command forwards selected scene/region, can stop, and discards a late result after scene changes', async () => {
  const request = deferred(), calls = [], stopped = [];
  const api = { extensionNative: true, subscribeExtensions: async () => () => {}, getExtensions: async () => [record()],
    invokeExtension: async input => { calls.push(input); return request.promise; }, stopExtension: async id => stopped.push(id) };
  let setRegion, props;
  reactive.createRoot(() => { const [regionId, set] = reactive.createSignal('region'); setRegion = set; props = { sceneId: 'scene', get regionId() { return regionId(); }, draft: 'Click the button', disabled: false, onCountChange() {}, onInteractionChange() {}, onRunningChange() {}, onError: message => { throw new Error(message); } }; });
  const { value, dispose } = await controller('./components/ExtensionSelectionTools.tsx', api, ['services', 'choose', 'running', 'stop', 'result', 'taskChoice'], props);
  await tick(); value.choose(value.services()[0]); await tick(); assert.equal(value.running().service.id, 'operate');
  assert.deepEqual(normalize(calls), [{ pluginId: manifest.id, revision: 1, serviceId: 'operate', input: { task: 'Click the button' }, sceneId: 'scene', regionId: 'region' }]);
  await value.stop(); assert.deepEqual(stopped, [manifest.id]); setRegion('different'); await tick(); request.resolve({ text: 'old answer' }); await tick();
  assert.equal(value.result(), ''); assert.equal(value.running(), undefined); dispose();
});

test('empty region draft requests only a task, and disposed listener initialization is released', async () => {
  const listen = deferred(); let stops = 0, reads = 0;
  const api = { extensionNative: true, subscribeExtensions: () => listen.promise, getExtensions: async () => { reads++; return []; } };
  const props = { sceneId: 'scene', regionId: 'region', draft: '', disabled: false, onCountChange() {}, onInteractionChange() {}, onRunningChange() {}, onError() {} };
  const { dispose } = await controller('./components/ExtensionSelectionTools.tsx', api, ['choose', 'taskChoice'], props);
  dispose(); listen.resolve(() => stops++); await tick(); assert.equal(stops, 1); assert.equal(reads, 0);
});

test('provider clear uses binding CAS and returns to host default; disabled bound provider remains visibly unavailable', async () => {
  const linked = { agentId: 'agent', kind: 'agent', pluginId: manifest.id, serviceId: 'engine', revision: 3 }, calls = [];
  let receive;
  const api = { extensionNative: true, subscribeExtensions: async callback => { receive = callback; return () => {}; }, subscribeExtensionBindings: async () => () => {},
    getExtensions: async () => [record({ services: [{ kind: 'agent', id: 'engine', title: 'Independent agent' }] })], getExtensionBindings: async () => [linked],
    setExtensionBinding: async (...args) => { calls.push(args); return [{ ...linked, pluginId: null, serviceId: null, revision: 4 }]; } };
  const { value, dispose } = await controller('./components/ExtensionProviders.tsx', api, ['selected', 'unavailable', 'select', 'binding', 'error'], { active: true, saved: true, agentId: 'agent' });
  await tick(); assert.equal(value.unavailable('agent'), false); receive([record({ revision: 2, state: 'disabled' })]); assert.equal(value.unavailable('agent'), true);
  await value.select('agent', ''); assert.deepEqual(normalize(calls), [['agent', 'agent', 3, undefined].map(value => value === undefined ? null : value)]);
  assert.equal(value.selected('agent'), ''); assert.equal(value.unavailable('agent'), false); assert.equal(value.binding('agent').revision, 4); dispose();
});
