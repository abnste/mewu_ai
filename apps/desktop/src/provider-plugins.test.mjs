// SPDX-License-Identifier: MPL-2.0
// Real manifest data, profile builder and ConnectionsPanel callbacks, synthetic IPC.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile, readdir } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';
const read = name => readFile(new URL(name, import.meta.url), 'utf8');
const plain = source => stripTypeScriptTypes(source, { mode: 'transform' });
const pure = async name => import(`data:text/javascript;base64,${Buffer.from(plain(await read(name))).toString('base64')}`);
const policy = await pure('./provider-presets.ts'), categories = await pure('./plugin-categories.ts');
const files = (await readdir(new URL('../plugins/', import.meta.url))).filter(name => /^official-provider-.*\.json$/.test(name));
const packages = await Promise.all(files.map(name => read(`../plugins/${name}`).then(JSON.parse)));
const presets = policy.bundledProviderPresets(packages);
const oldPresets = JSON.parse(await read('../providers/presets.json'));
const clean = value => JSON.parse(JSON.stringify(value));
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { resolve, reject, promise }; };

test('18 real official adapter packages preserve all 23 original typed templates', () => {
  assert.equal(packages.length, 18); assert.equal(presets.length, 23);
  assert.equal(new Set(presets.map(value => value.id)).size, 23);
  assert.deepEqual(presets.map(({ pluginId, pluginRevision, contributionId, ...value }) => value).sort((a,b) => a.id.localeCompare(b.id)), oldPresets.sort((a,b) => a.id.localeCompare(b.id)));
  assert.deepEqual(new Set(presets.map(value => value.advanced.protocol)), new Set(['chat_completions', 'anthropic_messages', 'openai_responses']));
  for (const value of packages) {
    assert.ok(value.contributions.length > 0 && value.contributions.length <= 16);
    assert.deepEqual(categories.pluginModuleTags(value), ['model-providers']);
    for (const contribution of value.contributions) assert.equal(contribution.kind, 'model.connection');
  }
  for (const value of presets) {
    assert.equal(value.pluginRevision, 1);
    assert.ok(value.pluginId.startsWith('mewu.provider.'));
    assert.ok(packages.find(packageValue => packageValue.id === value.pluginId).contributions.some(contribution => contribution.id === value.contributionId));
  }
});

test('selected template produces independent credential-free user draft with exact protocol/options', () => {
  const preset = structuredClone(presets.find(value => value.id === 'Anthropic'));
  preset.advanced.requestParameters = { temperature: 0.7, top_p: 0.8 };
  const profile = policy.profileFromProviderPreset(preset, 'new-connection', 'My Claude');
  assert.deepEqual(profile.advanced, preset.advanced);
  assert.equal(profile.providerId, 'Anthropic'); assert.equal(profile.hasKey, false); assert.equal(profile.credentialId, null); assert.equal(profile.revision, 0);
  assert.equal('pluginId' in profile, false);
  preset.advanced.requestParameters.temperature = 1; preset.baseUrl = 'https://changed.invalid';
  assert.equal(profile.advanced.requestParameters.temperature, 0.7); assert.equal(profile.baseUrl, 'https://api.anthropic.com/v1');
  profile.advanced.requestParameters.top_p = 0.3; assert.equal(preset.advanced.requestParameters.top_p, 0.8);
});

async function panelHarness(savedConnections = []) {
  const source = await read('./components/ConnectionsPanel.tsx');
  const tree = parse(source, { sourceType:'module', plugins:['typescript','jsx'] });
  const component = tree.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration;
  const body = component.body.body.filter(node => node.type !== 'ReturnStatement').map(node => source.slice(node.start,node.end)).join('\n');
  const calls = [], requests = [], cleanups = [];
  let pluginCallback, stopped = false;
  const props = { connections: structuredClone(savedConnections), defaultConnectionId: savedConnections[0]?.id ?? null, active:false };
  const document = { addEventListener(){}, removeEventListener(){}, getElementById(){return null;} };
  const scope = vm.createContext({
    props, document, crypto: { randomUUID: () => 'synthetic-draft' }, structuredClone, queueMicrotask, Element: class Element {},
    groupLabels: { china:'国内', global:'国际', custom:'自定义' }, message: cause => String(cause),
    createSignal(value) { return [() => value, next => value = typeof next === 'function' ? next(value) : next]; },
    createMemo: fn => fn, createEffect(){}, on(){}, onCleanup: fn => cleanups.push(fn),
    subscribePlugins: async callback => { pluginCallback = callback; return () => { stopped = true; }; },
    getProviderPresets: () => { const value = deferred(); requests.push(value); return value.promise; },
    cancelConnectionProbe: async id => calls.push(['cancel', id]),
    fromProfile: profile => ({ profile: structuredClone(profile), expectedRevision: profile.revision, dirty:false, isNew:false, parametersText:'{}' }),
    profileFromProviderPreset: policy.profileFromProviderPreset,
    t: value => value,
  });
  vm.runInContext(`${plain(body)}\nglobalThis.api = {loadPresets, add, ensure, presets, drafts, selected, presetLoading, presetError};`, scope);
  await tick();
  return { scope, props, calls, requests, api:scope.api, plugin:() => pluginCallback(), cleanup:() => cleanups.forEach(fn => fn()), stopped:() => stopped };
}

test('actual ConnectionsPanel rejects late template results and stale clicks after plugin change', async () => {
  const saved = { id:'saved-user', name:'Already saved', providerId:'Anthropic', baseUrl:'https://user.invalid/v1', model:'my-model', revision:7, credentialId:'private-credential', hasKey:true, advanced:{protocol:'anthropic_messages',authMode:'api_key',requestParameters:{}} };
  const harness = await panelHarness([saved]);
  harness.api.ensure(saved.id);
  const existingDraft = clean(harness.api.drafts()[saved.id]);
  harness.props.active = true;
  const oldRequest = harness.api.loadPresets(); assert.equal(harness.requests.length, 1);
  harness.plugin(); harness.plugin();
  const old = presets.find(value => value.id === 'Anthropic');
  harness.requests[0].resolve([old]); await oldRequest; await tick();
  assert.equal(harness.requests.length, 2); assert.equal(harness.api.presets().length, 0);
  harness.api.add(old); assert.equal(harness.api.drafts()['synthetic-draft'], undefined);
  const current = structuredClone(presets.find(value => value.id === 'OpenAIResponses'));
  harness.requests[1].resolve([current]); await tick();
  assert.deepEqual(clean(harness.api.presets()), [current]);
  harness.api.add(old); assert.equal(harness.api.drafts()['synthetic-draft'], undefined);
  harness.api.add(current); assert.equal(harness.api.drafts()['synthetic-draft'].profile.advanced.protocol, 'openai_responses');
  assert.deepEqual(clean(harness.api.drafts()[saved.id]), existingDraft); assert.deepEqual(harness.props.connections, [saved]);
  harness.plugin(); assert.equal(harness.requests.length, 3);
  assert.equal(harness.api.presets().length, 0); assert.equal(harness.api.drafts()['synthetic-draft'].profile.providerId, 'OpenAIResponses');
  harness.cleanup(); assert.equal(harness.stopped(), true);
  harness.requests[2].resolve([old]); await tick(); assert.equal(harness.api.presets().length, 0);
});

test('actual getter dispatches native registry IPC and browser derives real bundled package defaults', async () => {
  const source = await read('./bridge.ts'), tree = parse(source, {sourceType:'module',plugins:['typescript']});
  const declaration = tree.program.body.find(node => node.type === 'ExportNamedDeclaration' && node.declaration?.id?.name === 'getProviderPresets').declaration;
  const code = plain(source.slice(declaration.start, declaration.end));
  const calls = [], scope = vm.createContext({ native:true, invoke:async (...args) => { calls.push(args); return ['registry']; }, providerPackages:Object.fromEntries(packages.map((value,i) => [i,value])), bundledProviderPresets:policy.bundledProviderPresets });
  vm.runInContext(`${code};globalThis.get=getProviderPresets`, scope);
  assert.deepEqual(await scope.get(), ['registry']); assert.deepEqual(calls, [['get_provider_presets']]);
  scope.native = false; assert.deepEqual(clean(await scope.get()), presets); assert.equal(calls.length, 1);
});
