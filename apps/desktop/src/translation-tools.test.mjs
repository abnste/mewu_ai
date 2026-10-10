// node --experimental-vm-modules apps/desktop/src/translation-tools.test.mjs
// Isolated lifecycle/IPC/text contracts. No provider, screen or clipboard calls.
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
const source = async file => stripTypeScriptTypes(await readFile(new URL(file, import.meta.url), 'utf8'), { mode: 'transform' });
const plain = value => JSON.parse(JSON.stringify(value));
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const selection = new SourceTextModule(await source('ocr-selection.ts')); await selection.link(() => { throw Error('unexpected import'); }); await selection.evaluate();
const requests = new SourceTextModule(await source('translation-request.ts')); await requests.link(() => selection); await requests.evaluate();
const { TranslationRequests, translationScopeValid, translationLanguages } = requests.namespace;
const checks = [];
const target = { sceneId: 'scene-a', regionId: 'region', backgroundId: 'background', drawingRevision: 3, x: 10, y: 20, width: 800, height: 400 };
const request = (requestId, sceneId = 'scene-a') => ({ requestId, pluginId: 'mewu.translation', revision: 2, contributionId: 'translate', target: { ...target, sceneId }, language: 'zh-Hans' });
const scene = { id: 'scene-a', closed: false, frozen: false, connectionId: 'connection', background: { id: 'background' }, regions: [{ id: 'region', drawingRevision: 3, x: 10, y: 20, width: 800, height: 400 }] };
const snapshot = { scenes: [scene], connections: [{ id: 'connection', revision: 4 }] };
const plugins = [{ state: 'enabled', revision: 2, manifest: { id: 'mewu.translation', contributions: [{ id: 'translate', kind: 'selection.translation' }] } }];
assert.equal(translationScopeValid(request('a'), snapshot, plugins, 'connection:4'), true);
assert.equal(translationScopeValid(request('a'), { ...snapshot, activeSceneId: 'other', scenes: [{ ...scene, frozen: true }] }, plugins, 'connection:4'), true);
for (const changed of [{ ...scene, closed: true }, { ...scene, connectionId: 'other' }, { ...scene, background: { id: 'other' } }, { ...scene, regions: [] }, { ...scene, regions: [{ ...scene.regions[0], drawingRevision: 4 }] }, { ...scene, regions: [{ ...scene.regions[0], width: 700 }] }]) assert.equal(translationScopeValid(request('a'), { ...snapshot, scenes: [changed] }, plugins, 'connection:4'), false);
for (const changed of [{ ...plugins[0], state: 'disabled' }, { ...plugins[0], state: 'removed' }, { ...plugins[0], revision: 3 }, { ...plugins[0], error: 'invalid' }]) assert.equal(translationScopeValid(request('a'), snapshot, [changed], 'connection:4'), false);
assert.equal(translationScopeValid(request('a'), snapshot, plugins, 'connection:3'), false);
checks.push('Frozen/background scene stays valid; closed/source/geometry/revision/plugin/connection changes invalidate an in-flight request');

const operations = new Map(), canceled = [], results = [], errors = [], changes = [];
const gate = new TranslationRequests({ run: request => new Promise((resolve, reject) => operations.set(request.requestId, { resolve, reject })), cancel: async id => canceled.push(id), changed: values => changes.push(plain(values)), result: (value, request) => results.push([value, request.requestId]), error: (value, request) => errors.push([value, request.requestId]) });
const first = gate.start(request('first')); const replacement = gate.start(request('replacement')); const other = gate.start(request('other', 'scene-b'));
assert.deepEqual(canceled, ['first']); assert.equal(changes.at(-1).length, 2);
operations.get('first').resolve('late'); await first; assert.equal(results.length, 0); assert.ok(gate.has('replacement'));
operations.get('other').resolve('background'); await other; assert.deepEqual(results, [['background', 'other']]); assert.ok(gate.has('replacement'));
checks.push('Requests are owned per scene; replacing one cannot clear another, and canceled late completion is ignored');

const progress = { requestId: 'replacement', sceneId: 'scene-a', regionId: 'region', phase: 'translating', completed: 2, total: 5 };
gate.progress(progress); const count = changes.length;
for (const value of [{ ...progress, requestId: 'first' }, { ...progress, regionId: 'wrong' }, { ...progress, phase: 'recognizing' }, { ...progress, completed: 1 }, { ...progress, completed: 6 }, { ...progress, completed: NaN }, { ...progress, phase: 'unknown' }]) gate.progress(value);
assert.equal(changes.length, count); gate.progress({ ...progress, phase: 'rendering', completed: 5 }); assert.equal(changes.at(-1)[0].progress.phase, 'rendering');
gate.cancelScene('scene-a'); operations.get('replacement').reject(Error('late error')); await replacement; assert.equal(errors.length, 0);
checks.push('Progress requires current request/scene/region, finite counters and monotonic phase/count; cancel drops later progress and errors');

const fail = gate.start(request('fail')); operations.get('fail').reject(Error('服务错误')); await fail; assert.deepEqual(errors, [['服务错误', 'fail']]); assert.equal(changes.at(-1).length, 0);
const retry = gate.start(request('retry')); gate.retain(() => false); operations.get('retry').resolve('discard'); await retry; assert.ok(canceled.includes('retry'));
const disposed = gate.start(request('disposed')); gate.dispose(); operations.get('disposed').resolve('discard'); await disposed; await gate.start(request('never')); assert.equal(operations.has('never'), false);
checks.push('Failure restores idle/retry; retain cancellation and disposal cannot publish stale results');

async function bridgeFixture(native) {
  const calls = [], events = new Map(), stopped = [];
  const bridge = new SourceTextModule(await source('translation-bridge.ts'));
  await bridge.link(name => name.endsWith('/core') ? synthetic({ isTauri: () => native, invoke: async (command, args) => { calls.push({ command, args }); return { revision: 12 }; } }) : synthetic({ listen: async (name, callback) => { events.set(name, callback); return () => stopped.push(name); } }));
  await bridge.evaluate(); return { api: bridge.namespace, calls, events, stopped };
}
const native = await bridgeFixture(true), values = [], stop = await native.api.subscribeTranslation(value => values.push(value));
native.events.get('translation-progress')({ payload: progress }); assert.equal(values[0], progress);
await native.api.runTranslation(request('native')); await native.api.cancelTranslation('native'); await native.api.clearTranslation(target); stop();
assert.deepEqual(plain(native.calls), [{ command: 'run_plugin_translation', args: request('native') }, { command: 'cancel_plugin_translation', args: { requestId: 'native' } }, { command: 'clear_region_translation', args: { target } }]);
assert.deepEqual(native.stopped, ['translation-progress']);
const preview = await bridgeFixture(false); await assert.rejects(preview.api.runTranslation(request('no')), /桌面版/); await assert.rejects(preview.api.clearTranslation(target), /桌面版/); await preview.api.cancelTranslation('no'); assert.equal(preview.calls.length, 0);
checks.push('Native bridge passes exact IDs/target/language, explicit clear remains independent of enabled plugin; browser never simulates translation');

const doc = { engine: 'native-layout', language: 'zh-Hans', width: 500, height: 200, textAngle: 18, lines: [
  { text: '中文换行后也不插入空格 <svg> 👩‍💻', words: [{ text: '中文换行后', x: 10, y: 20, width: 120, height: 22 }, { text: '也不插入空格 <svg> 👩‍💻', x: 10, y: 45, width: 250, height: 22 }] },
  { text: '第二条原始行', words: [{ text: '第二条原始行', x: 10, y: 100, width: 150, height: 22 }] },
] };
const layout = selection.namespace.ocrLayout(doc);
assert.equal(layout.text, '中文换行后也不插入空格 <svg> 👩‍💻\n第二条原始行');
assert.ok(layout.glyphs.some(glyph => glyph.y === 45)); assert.ok(layout.glyphs.some(glyph => layout.text.slice(glyph.start, glyph.end) === '👩‍💻'));
assert.equal(translationLanguages.length, 7); assert.deepEqual(plain(translationLanguages.map(value => value[0])), ['zh-Hans', 'en', 'ja', 'ko', 'de', 'fr', 'es']);
checks.push('Native wrapped glyph segments preserve full original translated line text and Unicode; only original lines add newline');
const config = JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const capabilityDirectory = new URL('../src-tauri/capabilities/', import.meta.url);
const permissions = ['allow-run-plugin-translation', 'allow-cancel-plugin-translation', 'allow-clear-region-translation'];
for (const filename of await readdir(capabilityDirectory)) {
  if (!filename.endsWith('.json')) continue;
  const capability = JSON.parse(await readFile(new URL(filename, capabilityDirectory), 'utf8'));
  if (capability.identifier === 'space') { assert.ok(config.app.security.capabilities.includes(capability.identifier)); assert.deepEqual(capability.windows, ['space']); for (const value of permissions) assert.ok(capability.permissions.includes(value)); }
  else for (const value of permissions) assert.ok(!capability.permissions.includes(value), `${filename} must not receive translation IPC`);
}
checks.push('Actual Tauri configuration grants run/cancel/clear only to the enabled first-party space capability');
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
