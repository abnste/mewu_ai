// Synthetic grants, queue and actual bridge. No model/native/clipboard requests.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule, createContext } from 'node:vm';
import { webcrypto } from 'node:crypto';
import { loadProviderPresets, loadAssetImport, loadGeometryHistory } from './geometry-test-module.mjs';
const source = async name => stripTypeScriptTypes(await readFile(new URL(name, import.meta.url), 'utf8'), { mode: 'transform' });
const module = new SourceTextModule(await source('./visual-annotations-send.ts'));
await module.link(() => { throw Error('Unexpected import'); }); await module.evaluate();
const { annotationGrant, hasAnnotationTarget, annotationSendIdentity, assertAnnotationSend } = module.namespace;
const makePlugin = (id = 'mewu.annotations') => ({ manifest: { id, contributions: [{ id: 'annotate', kind: 'agent.visual-annotations', engine: 'host.vector-v1' }] }, revision: 3, state: 'enabled' });
const makeScene = () => ({ id: 'scene', agentId: 'agent', background: { id: 'background', width: 800, height: 600 }, connectionId: 'connection', regions: [{ id: 'region' }], items: [], refs: [{ kind: 'region', id: 'region' }], draft: '请在图中指出错误', messages: [] });
const checks = [];
{
  const plugins = [makePlugin('custom'), makePlugin()];
  assert.equal(annotationGrant(plugins).pluginId, 'mewu.core.annotations');
  plugins[1].state = 'disabled'; assert.equal(annotationGrant(plugins).pluginId, 'mewu.core.annotations');
  plugins[0].error = 'bad'; assert.equal(annotationGrant(plugins).pluginId, 'mewu.core.annotations');
  assert.equal(hasAnnotationTarget(makeScene(), [{ kind: 'item', id: 'region' }]), false);
  assert.equal(hasAnnotationTarget(makeScene(), [{ kind: 'region', id: 'gone' }]), false);
  assert.equal(hasAnnotationTarget(makeScene(), makeScene().refs), true);
  checks.push('Only live grants and actual referenced targets expose visual tools');
}
{
  const scene = makeScene(), plugin = makePlugin();
  const input = { scene, identity: annotationSendIdentity(scene), references: scene.refs, draft: scene.draft, grant: annotationGrant([plugin]), plugins: [plugin], active: true };
  assert.doesNotThrow(() => assertAnnotationSend(input));
  for (const changed of [{ id: 'other' }, { agentId: 'other' }, { connectionId: 'other' }, { background: { ...scene.background, path: 'replaced' } }, { run: { id: 'new', status: 'completed' } }, { closed: true }, { frozen: true }]) assert.throws(() => assertAnnotationSend({ ...input, scene: { ...scene, ...changed } }), /会话已变化/);
  assert.doesNotThrow(() => assertAnnotationSend({ ...input, plugins: [] }));
  assert.throws(() => assertAnnotationSend({ ...input, grant: { ...input.grant, pluginRevision: 2 } }), /标注能力已变化/);
  assert.doesNotThrow(() => assertAnnotationSend({ ...input, draft: ' ' }));
  assert.throws(() => assertAnnotationSend({ ...input, references: [] }), /引用截图选区或视频/);
  assert.throws(() => assertAnnotationSend({ ...input, active: false }), /会话已变化/);
  assert.doesNotThrow(() => assertAnnotationSend({ ...input, scene: { ...scene, draft: 'new local draft', updatedAt: 5 } }));
  checks.push('Scene/Agent/background metadata/connection/run/exit/grant fences reject stale sends without treating draft edits as source changes');
}
{
  const submission = new SourceTextModule(await source('./scene-submission.ts')); await submission.link(() => { throw Error('Unexpected'); }); await submission.evaluate();
  for (const changed of ['plugin', 'scene', 'failure', 'none']) {
    const scene = makeScene(), plugin = makePlugin(), grant = annotationGrant([plugin]), identity = annotationSendIdentity(scene); let modelCalls = 0; const order = [];
    const check = () => assertAnnotationSend({ identity, grant, scene, references: scene.refs, draft: scene.draft, plugins: [plugin], active: true });
    const run = submission.namespace.submitSceneDraft({ sceneId: scene.id, draft: scene.draft, refs: scene.refs }, {
      save: async command => { check(); order.push(command.type); await Promise.resolve(); if (command.type === 'set_refs') { if (changed === 'plugin') plugin.revision++; if (changed === 'scene') scene.id = 'other'; if (changed === 'failure') throw Error('save failed'); } },
      send: async () => { check(); modelCalls++; order.push('send'); return {}; },
    });
    if (changed === 'none' || changed === 'plugin') { await run; assert.equal(modelCalls, 1); assert.deepEqual(order, ['set_draft', 'set_refs', 'send']); }
    else { await assert.rejects(run); assert.equal(modelCalls, 0); assert.equal(scene.draft, '请在图中指出错误'); }
  }
  checks.push('Actual draft submission saves in order and rechecks after the last await; failures never reach the model or discard input');
}
{
  const context = createContext({ crypto: webcrypto, structuredClone, Date, Map, Set, Blob, TextEncoder, TextDecoder });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const calls = [], core = synthetic({ isTauri: () => true, invoke: async (command, args) => { calls.push({ command, args }); return { schemaVersion: 1, scenes: [], agents: [], connections: [] }; } });
  const connection = new SourceTextModule(await source('./connection-policy.ts'), { context }); await connection.link(() => { throw Error('Unexpected'); });
  const mosaic = new SourceTextModule(await source('./mosaic-preview.ts'), { context }); await mosaic.link(() => { throw Error('Unexpected'); });
  const assets = await loadAssetImport(context), geometry = await loadGeometryHistory(context);
  const bridge = new SourceTextModule(await source('./bridge.ts'), { context, initializeImportMeta: meta => { meta.glob = () => ({}); } });
  await bridge.link(async name => name === './provider-presets' ? await loadProviderPresets(context) : name === './asset-import' ? assets : name === './region-geometry-history' ? geometry : name === './mosaic-preview' ? mosaic : name === './connection-policy' ? connection : name.endsWith('/core') ? core : synthetic({ listen: async () => () => {} })); await bridge.evaluate();
  const grant = annotationGrant([makePlugin()]); await bridge.namespace.sendMessage('scene'); await bridge.namespace.sendMessage('scene', grant);
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{ command: 'send_message', args: { sceneId: 'scene', visualAnnotations: null } }, { command: 'send_message', args: { sceneId: 'scene', visualAnnotations: grant } }]);
  checks.push('Actual native bridge preserves absence or the exact revisioned optional tool capability');
}
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
