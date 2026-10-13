// SPDX-License-Identifier: MPL-2.0
// Actual product hook + Solid's browser reactive runtime; synthetic IPC only.
// Run in the project Docker image: node --experimental-vm-modules --test <file>.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { createHash } from 'node:crypto';

const reactive = await import(new URL('../../../node_modules/solid-js/dist/solid.js', import.meta.url));
const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const hookSource = await raw('./optional-feature-state.ts');
const policySource = await raw('./optional-capabilities.ts');
const compile = source => stripTypeScriptTypes(source, { mode: 'transform' });
const synthetic = values => new SyntheticModule(Object.keys(values), function () {
  for (const [name, value] of Object.entries(values)) this.setExport(name, value);
});
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const connectionId = '09fdbec2-556b-4e05-8b84-d5d0d9e403f0';
const otherConnectionId = 'bc704733-cdf0-4e2f-a7c7-0c21e3f1120b';
function connection(id = connectionId, revision = 1, configured = true) {
  return {
    id, revision, name: 'Synthetic connection', providerId: 'custom',
    baseUrl: 'https://example.invalid/v1', model: configured ? 'synthetic-model' : '',
    hasKey: configured, credentialId: configured ? 'd1ecb973-165f-452a-9693-90a7dbf02ac5' : null,
    advanced: { protocol: 'chat_completions', authMode: 'bearer', requestParameters: {} },
  };
}
function packageRecord(id, kind, state, revision) {
  return {
    manifest: {
      schemaVersion: 1, id, version: '1.0.0', name: id, description: 'Synthetic fixture',
      license: 'MPL-2.0', hostApi: 1,
      contributions: [{
        id: kind === 'agent.runtime' ? 'agent' : 'blackboard', title: id, kind,
        engine: kind === 'agent.runtime' ? 'host.agent-v1' : 'host.blackboard-v1',
      }],
    },
    source: { type: 'official' }, revision, state, hasRollback: false,
  };
}
function plugins(revision = 5, agentState = 'enabled', agentRevision = 1, boardState = 'disabled') {
  return {
    revision,
    plugins: [
      packageRecord('mewu.agent', 'agent.runtime', agentState, agentRevision),
      packageRecord('mewu.blackboard', 'space.blackboard', boardState, 1),
    ],
  };
}
const agentGrant = revision => ({ pluginId: 'mewu.agent', contributionId: 'agent', revision });
function capabilities(snapshot, connections, patch = {}) {
  const record = snapshot.plugins.find(value => value.manifest.id === 'mewu.agent');
  return {
    pluginRevision: snapshot.revision,
    connectionRevisions: Object.fromEntries(connections.map(value => [value.id, value.revision])),
    agent: record?.state === 'enabled' && !record.error ? agentGrant(record.revision) : null,
    blackboard: null, readyConnectionIds: [connectionId], ...patch,
  };
}
async function fixture({ initialPlugins = plugins(), supplied = true, delayedSubscription = false, initialConnections = [connection()] } = {}) {
  const reads = [], listeners = new Set(), subscription = deferred();
  let pluginReads = 0, subscribes = 0, stops = 0, currentPlugins = initialPlugins;
  const bridge = synthetic({
    getOptionalCapabilities: () => {
      const request = deferred(); reads.push(request); return request.promise;
    },
    getPlugins: async () => { pluginReads++; return currentPlugins; },
    subscribePlugins: async accept => {
      subscribes++; listeners.add(accept);
      let stopped = false;
      const stop = () => { if (!stopped) { stopped = true; stops++; listeners.delete(accept); } };
      if (delayedSubscription) { await subscription.promise; }
      return stop;
    },
  });
  const policy = new SourceTextModule(compile(policySource), { identifier: 'actual-optional-capabilities.ts' });
  await policy.link(specifier => { throw new Error('Unexpected product policy dependency: ' + specifier); });
  await policy.evaluate();
  const hook = new SourceTextModule(compile(hookSource), { identifier: 'actual-optional-feature-state.ts' });
  const solid = synthetic(reactive);
  await hook.link(specifier => {
    if (specifier === 'solid-js') return solid;
    if (specifier === './plugin-bridge') return bridge;
    if (specifier === './optional-capabilities') return policy;
    throw new Error('Unexpected product hook dependency: ' + specifier);
  });
  await hook.evaluate();
  let state, dispose, setSnapshot, setPlugins, readSnapshot, readPlugins;
  reactive.createRoot(cleanup => {
    dispose = cleanup;
    [readSnapshot, setSnapshot] = reactive.createSignal({ revision: 1, connections: initialConnections, scenes: [] });
    [readPlugins, setPlugins] = reactive.createSignal(initialPlugins);
    state = hook.namespace.createOptionalFeatureState(() => readSnapshot().connections, supplied ? readPlugins : undefined);
  });
  await tick();
  return {
    state, dispose, reads, setSnapshot, setPlugins,
    snapshot: readSnapshot, plugins: readPlugins,
    emit(snapshot) { currentPlugins = snapshot; for (const accept of [...listeners]) accept(snapshot); },
    releaseSubscription: () => subscription.resolve(),
    listenerCount: () => listeners.size, pluginReads: () => pluginReads,
    subscribes: () => subscribes, stops: () => stops,
    aiAvailable: () => Boolean(state()?.agent && state()?.readyConnectionIds.includes(connectionId)),
  };
}

test('actual hook never unlocks AI for unconfigured connections with empty native ready IDs', async t => {
  const f = await fixture({ initialConnections: [connection(connectionId, 1, false)] });
  t.after(f.dispose);
  assert.equal(f.reads.length, 1);
  assert.equal(f.state(), undefined);
  f.reads[0].resolve(capabilities(f.plugins(), f.snapshot().connections, { readyConnectionIds: [] }));
  await tick();
  assert.equal(f.state()?.agent?.pluginId, 'mewu.agent');
  assert.deepEqual(Array.from(f.state().readyConnectionIds), []);
  assert.equal(f.aiAvailable(), false);
  assert.equal(f.subscribes(), 0);
});

test('old connection and plugin responses cannot revive a newer configuration or overwrite its result', async t => {
  const f = await fixture();
  t.after(f.dispose);
  const firstPlugins = f.plugins(), firstConnections = f.snapshot().connections;
  f.setSnapshot({ revision: 2, connections: [connection(connectionId, 2)], scenes: [] });
  assert.equal(f.state(), undefined);
  await tick();
  assert.equal(f.reads.length, 2);
  const current = capabilities(f.plugins(), f.snapshot().connections);
  f.reads[1].resolve(current); await tick();
  assert.equal(f.state(), current);
  f.reads[0].resolve(capabilities(firstPlugins, firstConnections)); await tick();
  assert.equal(f.state(), current);
  const disabled = plugins(6, 'disabled', 2);
  f.setPlugins(disabled); assert.equal(f.state(), undefined);
  await tick();
  assert.equal(f.reads.length, 3);
  const removal = plugins(7, 'removed', 3);
  f.setPlugins(removal); await tick();
  assert.equal(f.reads.length, 4);
  const removedReceipt = capabilities(removal, f.snapshot().connections);
  f.reads[3].resolve(removedReceipt); await tick();
  assert.equal(f.state(), removedReceipt);
  f.reads[2].resolve(capabilities(disabled, f.snapshot().connections)); await tick();
  assert.equal(f.state(), removedReceipt);
  assert.equal(f.state().agent, null);
  assert.equal(f.aiAvailable(), false);
});

test('disabled, removed, malformed and mismatched native authorizations fail closed; valid null grants remain readable', async t => {
  for (const state of ['disabled', 'removed']) {
    const snapshot = plugins(8, state, 12);
    const f = await fixture({ initialPlugins: snapshot }); t.after(f.dispose);
    f.reads[0].resolve(capabilities(snapshot, f.snapshot().connections, { agent: agentGrant(12) }));
    await tick();
    assert.equal(f.state(), undefined, state + ' package must not authorize a live Agent');
    assert.equal(f.aiAvailable(), false);
    const next = plugins(9, state, 13);
    f.setPlugins(next); await tick();
    f.reads[1].resolve(capabilities(next, f.snapshot().connections)); await tick();
    assert.equal(f.state().agent, null);
    assert.equal(f.aiAvailable(), false);
  }
  for (const patch of [
    { agent: { ...agentGrant(1), revision: 0 } },
    { agent: { ...agentGrant(1), contributionId: 'missing' } },
    { agent: { ...agentGrant(1), revision: 2 } },
    { blackboard: { pluginId: 'mewu.blackboard', contributionId: 'blackboard', revision: 1 } },
    { pluginRevision: 4 },
    { connectionRevisions: { [connectionId]: 0 } },
    { connectionRevisions: [] },
    { readyConnectionIds: ['unknown-connection'] },
    { agent: undefined },
  ]) {
    const f = await fixture(); t.after(f.dispose);
    f.reads[0].resolve(capabilities(f.plugins(), f.snapshot().connections, patch)); await tick();
    assert.equal(f.state(), undefined);
    assert.equal(f.aiAvailable(), false);
  }
});

test('read errors close the current gate while a late error cannot erase a newer successful receipt', async t => {
  const f = await fixture(); t.after(f.dispose);
  f.reads[0].resolve(capabilities(f.plugins(), f.snapshot().connections)); await tick();
  assert.equal(f.aiAvailable(), true);
  f.setPlugins(plugins(6)); await tick();
  assert.equal(f.state(), undefined);
  f.reads[1].reject(new Error('Synthetic capability-read failure')); await tick();
  assert.equal(f.state(), undefined);
  assert.equal(f.aiAvailable(), false);
  f.setPlugins(plugins(7)); await tick();
  f.setPlugins(plugins(8)); await tick();
  const current = capabilities(f.plugins(), f.snapshot().connections);
  f.reads[3].resolve(current); await tick();
  f.reads[2].reject(new Error('Synthetic obsolete failure')); await tick();
  assert.equal(f.state(), current);
  assert.equal(f.aiAvailable(), true);
});

test('drawing and snapshot updates with unchanged configuration revisions never repeat credential reads', async t => {
  const f = await fixture({ initialConnections: [connection(), connection(otherConnectionId)] });
  t.after(f.dispose);
  const ready = capabilities(f.plugins(), f.snapshot().connections);
  f.reads[0].resolve(ready); await tick();
  assert.equal(f.aiAvailable(), true);
  for (let index = 0; index < 100; index++) {
    const connections = f.snapshot().connections.map(profile => ({ ...profile, advanced: { ...profile.advanced } }));
    if (index % 2) connections.reverse();
    f.setSnapshot({ revision: index + 2, connections, scenes: [{ drawingRevision: index + 1, messages: ['synthetic'] }] });
    await tick();
    assert.equal(f.reads.length, 1);
    assert.equal(f.state(), ready);
  }
  f.setSnapshot({ revision: 102, connections: [connection(connectionId, 2), connection(otherConnectionId)], scenes: [] });
  await tick();
  assert.equal(f.reads.length, 2, 'an actual profile revision must request a new host credential check');
  assert.equal(f.state(), undefined);
});

test('scope disposal ignores pending results and events and cleans both active and late subscription listeners', async t => {
  const f = await fixture({ supplied: false }); t.after(f.dispose);
  assert.equal(f.subscribes(), 1);
  assert.equal(f.pluginReads(), 1);
  assert.equal(f.listenerCount(), 1);
  assert.equal(f.reads.length, 1);
  f.dispose();
  assert.equal(f.listenerCount(), 0);
  assert.equal(f.stops(), 1);
  f.emit(plugins(6));
  f.reads[0].resolve(capabilities(plugins(), f.snapshot().connections));
  await tick();
  assert.equal(f.state(), undefined);
  assert.equal(f.reads.length, 1);
  assert.equal(f.stops(), 1);
  const pending = await fixture({ supplied: false, delayedSubscription: true }); t.after(pending.dispose);
  assert.equal(pending.subscribes(), 1);
  assert.equal(pending.pluginReads(), 0);
  assert.equal(pending.reads.length, 0);
  pending.dispose();
  pending.emit(plugins(9));
  pending.releaseSubscription(); await tick();
  assert.equal(pending.stops(), 1);
  assert.equal(pending.listenerCount(), 0);
  assert.equal(pending.pluginReads(), 0);
  assert.equal(pending.reads.length, 0);
  assert.equal(pending.state(), undefined);
});

console.log(JSON.stringify({
  verification: 'actual-product-hook-with-real-solid-reactivity-and-synthetic-ipc',
  hookSha256: createHash('sha256').update(hookSource).digest('hex'),
  policySha256: createHash('sha256').update(policySource).digest('hex'),
  nodeVersion: process.version,
  nativeCalls: 0, networkCalls: 0,
}));
