// SPDX-License-Identifier: MPL-2.0
// Actual draft helpers and ConnectionsPanel callbacks; synthetic cancellable IPC.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import { createContext, SourceTextModule, runInContext } from 'node:vm';

const read = file => readFile(new URL(file, import.meta.url), 'utf8');
const plain = value => stripTypeScriptTypes(value, { mode: 'transform' });
const clean = value => JSON.parse(JSON.stringify(value));
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const modules = new Map(), context = createContext({ structuredClone, URL });
async function load(file) {
  if (modules.has(file)) return modules.get(file);
  const module = new SourceTextModule(plain(await read(file)), { context, identifier: new URL(file, import.meta.url).href });
  modules.set(file, module);
  await module.link((specifier, owner) => load(new URL(specifier + (specifier.endsWith('.ts') ? '' : '.ts'), owner.identifier).href));
  await module.evaluate();
  return module;
}
const draft = (await load('./components/connection-draft.ts')).namespace;
const policy = (await load('./connection-policy.ts')).namespace;
function profile(baseUrl = 'https://api.example.test') {
  return { id: 'd362cb43-2704-44b8-87b0-9ee9e68c9a3f', providerId: 'custom', name: 'Synthetic', baseUrl, model: 'already-entered-model',
    hasKey: true, credentialId: 'synthetic-private-reference', revision: 7,
    advanced: { protocol: 'chat_completions', authMode: 'bearer', requestPath: null, requestParameters: { temperature: .4 } } };
}

test('context window is a host option, validated without entering provider JSON', () => {
  const value = profile();
  for (const window of [null, 8192, 65536, 2097152]) {
    value.advanced.contextWindowTokens = window;
    assert.doesNotThrow(() => policy.validateConnectionProfile(value));
  }
  for (const window of [0, 8191, 2097153, 1.5, Infinity, '65536']) {
    value.advanced.contextWindowTokens = window;
    assert.throws(() => policy.validateConnectionProfile(value));
  }
  assert.deepEqual(value.advanced.requestParameters, { temperature: .4 });
});

test('catalog correction changes only the exact draft URL and waits for explicit save', () => {
  const original = draft.connectionDraft(profile());
  original.key = 'synthetic-key-draft'; original.parametersText = '{ "temperature": 0.4 }';
  const before = clean(original), result = draft.applyModelCatalogToDraft(original, original, 'https://api.example.test/v1');
  assert.deepEqual(clean(original), before);
  assert.equal(result.profile.baseUrl, 'https://api.example.test/v1'); assert.equal(result.dirty, true);
  assert.deepEqual(clean({ ...result, dirty: before.dirty, profile: { ...result.profile, baseUrl: before.profile.baseUrl } }), before);
  assert.equal(draft.applyModelCatalogToDraft(original, original, original.profile.baseUrl), original);
  const newer = { ...original, profile: { ...original.profile, baseUrl: 'https://manual.example.test/v5' } };
  assert.equal(draft.applyModelCatalogToDraft(original, newer, 'https://api.example.test/v1'), newer);
});

test('catalog draft correction refuses other origins, explicit paths and malformed results', () => {
  for (const [base, custom, resolved] of [
    ['https://api.example.test', null, 'https://other.example.test/v1'],
    ['https://api.example.test/v2', null, 'https://api.example.test/v1'],
    ['https://api.example.test', '/custom/chat', 'https://api.example.test/v1'],
    ['https://api.example.test', null, 'https://api.example.test/other'],
    ['https://api.example.test', null, 'https://api.example.test/v1?key=synthetic'],
    ['https://api.example.test', null, 'http://api.example.test/v1'],
  ]) {
    const value = draft.connectionDraft(profile(base)); value.profile.advanced.requestPath = custom;
    assert.throws(() => draft.applyModelCatalogToDraft(value, value, resolved));
    assert.equal(value.profile.baseUrl, base); assert.equal(value.dirty, false);
  }
});

async function panel() {
  const source = await read('./components/ConnectionsPanel.tsx');
  const tree = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  const component = tree.program.body.find(n => n.type === 'ExportDefaultDeclaration').declaration;
  const body = component.body.body.filter(n => n.type !== 'ReturnStatement').map(n => source.slice(n.start, n.end)).join('\n');
  const calls = [], flights = [], canceled = [], cleanups = [], keyReads = [];
  const saved = profile(), props = { connections: [saved], defaultConnectionId: saved.id, active: true,
    onSave: async (...args) => calls.push(args), onDefault: async () => {}, onDelete: async () => {} };
  const scope = createContext({
    props, URL, structuredClone, crypto: { randomUUID: () => `probe-${flights.length + 1}` },
    document: { addEventListener() {}, removeEventListener() {} }, Element: class Element {},
    createSignal(value) { return [() => value, next => value = typeof next === 'function' ? next(value) : next]; },
    createMemo: fn => fn, createEffect() {}, on() {}, onCleanup: fn => cleanups.push(fn),
    subscribePlugins: async () => () => {}, getProviderPresets: async () => [],
    readConnectionKey: (id, revision) => { const flight = { ...deferred(), id, revision }; keyReads.push(flight); return flight.promise; },
    discoverConnectionModels: args => { const flight = { ...deferred(), args }; flights.push(flight); return flight.promise; },
    cancelConnectionProbe: async id => canceled.push(id), testConnection: async () => { throw Error('unexpected model request'); },
    fromProfile: draft.connectionDraft, applyModelCatalogToDraft: draft.applyModelCatalogToDraft, settleConnectionSave: draft.settleConnectionSave,
    validateConnectionProfile: policy.validateConnectionProfile, normalizeConnectionPath: policy.normalizeConnectionPath, parseConnectionParameters: policy.parseConnectionParameters,
    t: value => value, groupLabels: {}, message: cause => cause instanceof Error ? cause.message : String(cause),
  });
  runInContext(plain(body) + '\nglobalThis.api={open,ensure,field,edit,form,runProbe,save,models,modelStatus,errors,probe};', scope);
  await tick(); scope.api.open(saved.id);
  return { api: scope.api, props, id: saved.id, calls, flights, canceled, keyReads, dispose: () => cleanups.forEach(f => f()) };
}

test('saved key is refilled as its real password value; unchanged save does not replace the encrypted key', async () => {
  const f = await panel(), actualKey = 'a123456789012345';
  assert.equal(f.keyReads.length, 1);
  assert.equal(f.keyReads[0].revision, 7);
  f.keyReads[0].resolve(actualKey); await tick();
  assert.equal(f.api.form(f.id).key, actualKey);
  assert.equal(f.api.form(f.id).key.length, 16);
  assert.equal(f.api.form(f.id).keyEdited, false);
  f.props.onSave = async (...args) => { f.calls.push(args); f.props.connections[0] = { ...args[0], revision: 8 }; };
  await f.api.save(f.id);
  assert.equal(f.calls[0][2], undefined);
  f.keyReads.at(-1).resolve(actualKey); await tick();
  f.api.edit(f.id, { key: actualKey + 'x' });
  await f.api.save(f.id);
  assert.equal(f.calls[1][2], actualKey + 'x');
  f.dispose();
});

test('late saved-key reads cannot replace typed input, another profile, another revision or a disposed editor', async () => {
  for (const action of ['edit', 'switch', 'revision', 'dispose']) {
    const f = await panel();
    if (action === 'edit') f.api.edit(f.id, { key: 'typed-replacement' });
    else if (action === 'switch') { f.props.connections.push({ ...profile(), id: 'another' }); f.api.open('another'); }
    else if (action === 'revision') f.api.edit(f.id, { expectedRevision: 8 });
    else f.dispose();
    f.keyReads[0].resolve('a123456789012345'); await tick();
    if (action !== 'dispose') { assert.equal(f.api.form(f.id).key, action === 'edit' ? 'typed-replacement' : ''); f.dispose(); }
  }
});

test('an own-save snapshot already refreshed by the reactive subscriber stays clean without discarding subsequent edits', () => {
  const submitted = draft.connectionDraft(profile()), latest = { ...profile(), revision: 8 };
  const refreshed = draft.connectionDraft(latest);
  assert.equal(draft.settleConnectionSave(submitted, refreshed, latest).dirty, false);
  const edited = { ...refreshed, dirty: true, profile: { ...latest, model: 'new-draft-model' } };
  const result = draft.settleConnectionSave(submitted, edited, latest);
  assert.equal(result.dirty, true); assert.equal(result.profile.model, 'new-draft-model');
});

test('actual panel accepts corrected base visibly, preserves model/key and never saves on discovery', async () => {
  const f = await panel();
  f.api.edit(f.id, { key: 'synthetic-key-draft', parametersText: '{"temperature":0.4}' });
  const pending = f.api.runProbe(f.id, 'models');
  assert.equal(f.flights.length, 1); assert.equal(f.flights[0].args.profile.model, 'already-entered-model');
  f.flights[0].resolve({ models: ['kept', 'other'], resolvedBaseUrl: 'https://api.example.test/v1' }); await pending;
  const current = f.api.form(f.id);
  assert.equal(current.profile.baseUrl, 'https://api.example.test/v1'); assert.equal(current.profile.model, 'already-entered-model');
  assert.equal(current.key, 'synthetic-key-draft'); assert.equal(current.expectedRevision, 7); assert.equal(current.dirty, true);
  assert.deepEqual(clean(f.api.models()[f.id]), ['kept', 'other']); assert.equal(f.calls.length, 0);
  assert.equal(f.props.connections[0].baseUrl, 'https://api.example.test');
  await f.api.save(f.id); assert.equal(f.calls.length, 1); assert.equal(f.calls[0][0].baseUrl, 'https://api.example.test/v1');
  f.dispose();
});

test('actual panel model-first/incomplete-URL edits retain fields and do not start automatic discovery', async () => {
  const f = await panel();
  f.api.field(f.id, { model: 'typed-first' });
  for (const baseUrl of ['h', 'https:', 'https://', '']) f.api.field(f.id, { baseUrl });
  assert.equal(f.flights.length, 0); assert.equal(f.api.form(f.id).profile.model, 'typed-first');
  await f.api.runProbe(f.id, 'models'); assert.equal(f.flights.length, 0); assert.ok(f.api.errors()[f.id]);
  f.api.field(f.id, { baseUrl: 'https://api.example.test' });
  const pending = f.api.runProbe(f.id, 'models'); f.flights[0].reject(Error('服务返回的模型列表格式无效')); await pending;
  assert.equal(f.api.form(f.id).profile.baseUrl, 'https://api.example.test'); assert.equal(f.api.form(f.id).profile.model, 'typed-first');
  assert.equal(f.calls.length, 0); f.dispose();
});

test('actual panel late catalog cannot replace a manually edited draft, another connection or a closed panel', async () => {
  for (const action of ['edit', 'revision', 'switch', 'inactive', 'dispose']) {
    const f = await panel(), pending = f.api.runProbe(f.id, 'models');
    if (action === 'edit') f.api.field(f.id, { baseUrl: 'https://manual.example.test/v3' });
    else if (action === 'revision') f.props.connections[0] = { ...f.props.connections[0], revision: 8 };
    else if (action === 'switch') { f.props.connections.push({ ...profile(), id: 'other-connection' }); f.api.open('other-connection'); }
    else if (action === 'inactive') f.props.active = false;
    else f.dispose();
    f.flights[0].resolve({ models: ['stale'], resolvedBaseUrl: 'https://api.example.test/v1' }); await pending;
    if (action !== 'dispose') {
      assert.equal(f.api.form(f.id).profile.baseUrl, action === 'edit' ? 'https://manual.example.test/v3' : 'https://api.example.test');
      assert.equal(f.api.models()[f.id], undefined); f.dispose();
    }
    assert.equal(f.calls.length, 0);
  }
});

test('actual discovery bridge returns resolvedBaseUrl without saving or making browser requests', async () => {
  const source = await read('./bridge.ts'), tree = parse(source, { sourceType: 'module', plugins: ['typescript'] });
  const node = tree.program.body.find(n => n.type === 'ExportNamedDeclaration' && n.declaration?.id?.name === 'discoverConnectionModels').declaration;
  const actual = plain(source.slice(node.start, node.end));
  const calls = [], result = { models: ['model'], resolvedBaseUrl: 'https://api.example.test/v1' };
  const native = createContext({ native: true, invoke: async (...args) => { calls.push(clean(args)); return result; } });
  runInContext(actual + '\nglobalThis.request = discoverConnectionModels;', native);
  const value = { requestId: 'catalog', profile: profile() };
  assert.equal(await native.request(value), result);
  assert.deepEqual(calls, [['discover_connection_models', { ...value, expectedRevision: null, apiKey: null }]]);
  const browser = createContext({ native: false, invoke: () => { throw Error('unexpected native request'); } });
  runInContext(actual + '\nglobalThis.request = discoverConnectionModels;', browser);
  await assert.rejects(browser.request(value), /桌面版获取模型/);
  assert.equal(calls.length, 1);
});
