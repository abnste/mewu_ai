// node --experimental-vm-modules apps/desktop/src/voice-input.test.mjs
// Synthetic promises and IPC only. Does not access any audio device.
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
const source = async file => stripTypeScriptTypes(await readFile(new URL(file, import.meta.url), 'utf8'), { mode: 'transform' });
const mod = new SourceTextModule(await source('voice-input.ts')); await mod.link(() => { throw Error('unexpected import'); }); await mod.evaluate();
const { VoiceInput, appendDictation, voiceLanguages } = mod.namespace;
const checks = [], tick = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const scope = () => ({ sceneId: 'scene', agentId: 'agent', backgroundId: 'background', runId: null, pluginId: 'mewu.voice', revision: 1, contributionId: 'dictation' });
function fixture() {
  const runs = [], stops = [], changes = [], errors = [], writes = [];
  let active = scope(), text = 'A', composing = false, nonce = 0, pending;
  const input = new VoiceInput({ current: () => active, run: request => { const wait = deferred(); runs.push({ ...wait, request }); return wait.promise; }, cancel: id => { const wait = deferred(); stops.push({ ...wait, id }); return wait.promise; }, composing: () => composing, draft: () => text, setDraft: value => { text = value; writes.push(value); }, changed: value => { changes.push(value); pending = value; }, error: error => errors.push(String(error)), id: () => `request-${++nonce}` });
  const result = (i, speech = '语音', extra = {}) => ({ requestId: runs[i].request.requestId, sceneId: runs[i].request.sceneId, text: speech, language: 'zh-CN', confidence: 'recognized', ...extra });
  return { input, runs, stops, changes, errors, writes, result, draft: () => text, setDraft: value => text = value, pending: () => pending, scope: value => active = value, composing: value => composing = value };
}
assert.equal(appendDictation('保留空格  ', '  识别文字  '), '保留空格  识别文字');
assert.equal(appendDictation('最新草稿', '补充'), '最新草稿 补充');
assert.equal(appendDictation('', '原样 <svg>'), '原样 <svg>');
for (const text of ['', '  ', 'x\0y', '😀'.repeat(8001), ' '.repeat(8001) + 'x']) assert.throws(() => appendDictation('old', text));
assert.equal(appendDictation('', '😀'.repeat(8000)).length, 16000);
assert.deepEqual(voiceLanguages({ supported: true, languages: [{ tag: 'zh-CN', name: '中文' }] }).map(x => x.value), ['system', 'zh-CN']);
assert.deepEqual(voiceLanguages({ supported: false, languages: [] }), []);
checks.push('Bounded Unicode/literal append preserves current text; installed Chinese does not imply English support');

{
  const f = fixture(); assert.equal(f.input.start('system'), true); assert.equal(f.input.start('en-US'), false);
  f.setDraft('B'); f.runs[0].resolve(f.result(0)); await tick();
  assert.equal(f.draft(), 'B 语音'); assert.deepEqual(f.writes, ['B 语音']); assert.equal(f.pending(), undefined); assert.equal(f.stops.length, 0);
  assert.equal(f.input.start('zh-CN'), true); f.runs[1].reject(Error('设备不可用')); await tick();
  assert.match(f.errors[0], /设备不可用/); assert.equal(f.pending(), undefined); assert.equal(f.draft(), 'B 语音');
  checks.push('Single request merges into latest draft once and failure restores retry without submitting a message');
}
{
  const f = fixture(); f.composing(true); f.input.start('system'); f.runs[0].resolve(f.result(0, '候选', { confidence: 'candidate' })); await tick();
  assert.equal(f.draft(), 'A'); assert.equal(f.pending().awaitingComposition, true);
  f.setDraft('已经完成组词'); f.composing(false); f.input.compositionEnded(); await tick();
  assert.equal(f.draft(), '已经完成组词 候选'); assert.equal(f.pending(), undefined); f.input.compositionEnded(); await tick(); assert.equal(f.writes.length, 1);
  checks.push('IME result waits and merges once after finalized composition without replacing the composed draft');
}
{
  const f = fixture(); f.input.start('system'); const canceled = f.input.cancel();
  assert.equal(f.input.isActive('request-1'), false); assert.equal(f.pending().phase, 'stopping');
  f.runs[0].resolve(f.result(0, '迟到的候选', { confidence: 'candidate' })); await tick();
  assert.equal(f.draft(), 'A'); assert.equal(f.input.start('system'), false); assert.equal(f.stops.length, 1);
  f.stops[0].resolve(); await canceled; assert.equal(f.pending(), undefined); assert.equal(f.input.start('system'), true);
  f.input.invalidate('request-1'); await tick(); assert.equal(f.stops.length, 1); assert.equal(f.input.isActive('request-2'), true);
  f.runs[1].resolve(f.result(1)); await tick();
  checks.push('Cancel revokes synchronously, late candidate cannot insert, slot awaits actual cancel, old invalidation cannot cancel a new request');
}
{
  const f = fixture(); f.input.start('system'); const canceled = f.input.cancel(); await tick(); f.stops[0].resolve(); await tick();
  assert.equal(f.input.start('system'), false); f.runs[0].reject(Error('native canceled')); await canceled;
  assert.equal(f.pending(), undefined); assert.equal(f.errors.length, 0);
  checks.push('Successful cancel acknowledgement alone cannot release the slot before the original run actually settles');
}
{
  const f = fixture(); f.input.start('system'); const canceled = f.input.cancel(); await tick(); f.stops[0].reject(Error('麦克风未关闭')); f.runs[0].resolve(f.result(0));
  await assert.rejects(canceled, /麦克风未关闭/); assert.equal(f.draft(), 'A');
  checks.push('Cancellation failure rejects the awaiting send/transition instead of silently proceeding');
}
{
  const f = fixture(); f.composing(true); f.input.start('system'); f.runs[0].resolve(f.result(0)); await tick();
  const canceled = f.input.cancel(); f.composing(false); f.input.compositionEnded(); await tick(); f.stops[0].resolve(); await canceled;
  assert.equal(f.draft(), 'A'); assert.equal(f.writes.length, 0);
  checks.push('Exit/stop while IME is composing discards the parked result before a later compositionend');
}
{
  for (const replacement of [undefined, { ...scope(), sceneId: 'new-scene' }, { ...scope(), agentId: 'new-agent' }, { ...scope(), backgroundId: 'new-background' }, { ...scope(), runId: 'new-completed-run' }, { ...scope(), revision: 2 }, { ...scope(), contributionId: 'different' }]) {
    const f = fixture(); f.input.start('system'); f.scope(replacement); f.input.reconcile(); await tick();
    assert.equal(f.input.isActive('request-1'), false); assert.equal(f.stops.length, 1);
    f.runs[0].resolve(f.result(0)); f.stops[0].resolve(); await tick(); assert.equal(f.writes.length, 0);
  }
  const f = fixture(); f.input.start('system'); f.input.dispose(); await tick(); f.stops[0].resolve(); f.runs[0].resolve(f.result(0)); await tick(); assert.equal(f.writes.length, 0); assert.equal(f.input.start('system'), false);
  checks.push('Scene/Agent/plugin/scope revocation and component disposal reject all late writes');
}
{
  const f = fixture(); f.input.start('system');
  f.input.progress({ requestId: 'request-1', sceneId: 'scene', phase: 'listening' }); assert.equal(f.pending().phase, 'listening');
  for (const value of [{ requestId: 'request-1', sceneId: 'scene', phase: 'starting' }, { requestId: 'old', sceneId: 'scene', phase: 'stopping' }, { requestId: 'request-1', sceneId: 'other', phase: 'stopping' }]) f.input.progress(value);
  assert.equal(f.pending().phase, 'listening'); f.runs[0].resolve(f.result(0, 'bad', { requestId: 'old' })); await tick(); assert.equal(f.writes.length, 0); assert.match(f.errors[0], /失效/);
  checks.push('Progress has exact request/scene and monotonic phase; mismatched returned result never inserts');
}
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
async function bridgeFixture(native, failListen = false) {
  const calls = [], events = new Map(), stopped = [];
  const bridge = new SourceTextModule(await source('voice-bridge.ts'));
  await bridge.link(name => name.endsWith('/core') ? synthetic({ isTauri: () => native, invoke: async (name, args) => { calls.push({ name, args }); return {}; } }) : synthetic({ listen: async (name, callback) => { if (failListen && name === 'voice-invalidated') throw Error('listen failed'); events.set(name, callback); return () => stopped.push(name); } }));
  await bridge.evaluate(); return { api: bridge.namespace, calls, events, stopped };
}
{
  const f = await bridgeFixture(true), progress = [], invalidated = [];
  const stop = await f.api.subscribeVoice(value => progress.push(value), id => invalidated.push(id));
  f.events.get('voice-progress')({ payload: { requestId: 'one', sceneId: 'scene', phase: 'listening' } }); f.events.get('voice-invalidated')({ payload: { requestId: 'one' } });
  const request = { requestId: 'one', pluginId: 'mewu.voice', revision: 2, contributionId: 'dictation', sceneId: 'scene', language: 'system' };
  await f.api.getVoiceCapabilities(); await f.api.runDictation(request); await f.api.cancelDictation('one'); stop();
  assert.deepEqual(f.calls, [{ name: 'get_voice_capabilities', args: undefined }, { name: 'run_plugin_dictation', args: request }, { name: 'cancel_plugin_dictation', args: { requestId: 'one' } }]);
  assert.deepEqual(invalidated, ['one']); assert.equal(progress.length, 1); assert.deepEqual(f.stopped, ['voice-progress', 'voice-invalidated']);
  const failed = await bridgeFixture(true, true); await assert.rejects(failed.api.subscribeVoice(() => {}, () => {})); assert.deepEqual(failed.stopped, ['voice-progress']);
  const browser = await bridgeFixture(false); assert.equal((await browser.api.getVoiceCapabilities()).supported, false); await assert.rejects(browser.api.runDictation(request), /桌面版/); await browser.api.cancelDictation('one'); assert.equal(browser.calls.length, 0);
  checks.push('Native bridge exact wire and partial-listener cleanup; browser has no microphone request or fabricated success');
}
{
  const config = JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  const directory = new URL('../src-tauri/capabilities/', import.meta.url);
  for (const file of await readdir(directory)) {
    if (!file.endsWith('.json')) continue;
    const capability = JSON.parse(await readFile(new URL(file, directory), 'utf8'));
    const permissions = capability.permissions.map(value => typeof value === 'string' ? value : value.identifier);
    if (capability.windows.includes('space')) {
      assert.ok(config.app.security.capabilities.includes(capability.identifier));
      for (const name of ['allow-get-voice-capabilities', 'allow-run-plugin-dictation', 'allow-cancel-plugin-dictation']) assert.ok(permissions.includes(name));
    } else {
      assert.ok(!permissions.includes('allow-run-plugin-dictation')); assert.ok(!permissions.includes('allow-cancel-plugin-dictation'));
      if (permissions.includes('allow-get-voice-capabilities')) assert.deepEqual(capability.windows, ['settings']);
    }
  }
  checks.push('Active Tauri capability list exposes microphone run/cancel only to space; settings may only enumerate');
}
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
