// node --test apps/desktop/src/recording-bundle.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
const read = name => readFile(new URL(name, import.meta.url), 'utf8');
const moduleOf = async name => import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await read(name), { mode: 'transform' })).toString('base64')}`);
const audio = await moduleOf('./recording-audio.ts'), categories = await moduleOf('./plugin-categories.ts');
// Retired package records remain user data. Core capture must not inherit
// their installed/enabled state or accept a developer package's authority.
const manifest = { id: 'mewu.recording', contributions: [
  { id: 'record', kind: 'selection.recording' }, { id: 'audio', kind: 'recording.audio' },
  { id: 'trim', kind: 'artifact.video-trim' }, { id: 'gif', kind: 'artifact.video-gif' },
] };
const plugin = (changes = {}) => ({ manifest: structuredClone(manifest), source: { type: 'official' }, state: 'enabled', revision: 4, hasRollback: false, ...changes });
const base = { pluginId: 'mewu.core.recording', revision: 1, contributionId: 'record' }, sound = { ...base, contributionId: 'audio' };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const turn = () => new Promise(resolve => setImmediate(resolve));
async function functionFrom(file, name, context) {
  const text = await read(file), tree = parse(text, { sourceType: 'module', plugins: ['typescript', 'jsx'] }); let declaration;
  function visit(node) { if (!node || typeof node !== 'object') return; if (node.type === 'FunctionDeclaration' && node.id?.name === name) declaration = node; else for (const child of Object.values(node)) Array.isArray(child) ? child.forEach(visit) : visit(child); } visit(tree);
  assert.ok(declaration, `${name} exists`);
  const code = stripTypeScriptTypes(text.slice(declaration.start, declaration.end), { mode: 'transform' });
  return new Function(...Object.keys(context), `${code};return ${name}`)(...Object.values(context));
}

test('actual helpers expose complete core recording with an empty plugin inventory', () => {
  assert.deepEqual(categories.pluginCategories(manifest), ['selection.recording']);
  assert.deepEqual(audio.recordingGrant(), base);
  assert.deepEqual(audio.recordingAudioGrants(), [sound]);
  assert.deepEqual(audio.recordingTrimGrant(), { ...base, contributionId: 'trim' });
  assert.deepEqual(audio.recordingGifGrant(), { ...base, contributionId: 'gif' });
  assert.equal(audio.validRecordingTrimGrant({ ...base, contributionId: 'trim' }), true);
  for (const bad of [{ ...base, contributionId: 'audio' }, { ...base, contributionId: 'trim', revision: 2 }, { ...base, contributionId: 'trim', pluginId: 'developer.recording' }]) assert.equal(audio.validRecordingTrimGrant(bad), false);
});

test('external, orphan and spoofed packages cannot override precise core capabilities', () => {
  const external = plugin({ manifest: { ...manifest, id: 'developer.recording' }, revision: 7 });
  assert.deepEqual(audio.recordingGrant([external, plugin()]), base);
  assert.deepEqual(audio.recordingAudioGrants([external, plugin()]), [sound]);
  const baseOnly = plugin({ manifest: { ...manifest, contributions: manifest.contributions.filter(value => value.kind === 'selection.recording') } });
  assert.deepEqual(audio.recordingAudioGrants([external, baseOnly]), [sound]);
  const orphanAudio = plugin({ manifest: { ...manifest, contributions: manifest.contributions.filter(value => value.kind === 'recording.audio') } });
  assert.deepEqual(audio.recordingGrant([orphanAudio]), base);
  assert.deepEqual(audio.recordingAudioGrants([orphanAudio]), [sound]);
  const spoofed = plugin({ manifest: { ...manifest, id: base.pluginId }, revision: 55, source: { type: 'github' } });
  assert.deepEqual(audio.recordingGrant([spoofed]), base);
  assert.deepEqual(audio.recordingTrimGrant([spoofed]), { ...base, contributionId: 'trim' });
  assert.deepEqual(audio.recordingAudioGrants([spoofed]), [sound]);
});

test('retired package state cannot disable core audio, but an unknown core revision never authorizes it', () => {
  const controller = new audio.RecordingAudioController(async () => assert.fail('No implicit save'), () => {});
  controller.reconcile(audio.recordingAudioGrants([plugin()]));
  controller.accept({ revision: 2, sequence: 2, mode: 'both', grant: sound, editable: true });
  assert.equal(controller.selection().mode, 'both');
  for (const changed of [{ state: 'disabled' }, { state: 'removed' }, { error: 'invalid manifest' }, { revision: 6 }]) {
    const record = plugin(changed);
    assert.deepEqual(audio.recordingGrant([record]), base);
    assert.deepEqual(audio.recordingTrimGrant([record]), { ...base, contributionId: 'trim' });
    controller.reconcile(audio.recordingAudioGrants([record]));
    assert.equal(controller.selection().mode, 'both');
  }
  controller.accept({ revision: 3, sequence: 3, mode: 'both', grant: { ...sound, revision: 2 }, editable: true });
  assert.equal(controller.view().mode, 'both'); assert.throws(() => controller.selection(), /请重新选择声源/);
});

test('unread System is presentation only; a native receipt authorizes it and saved mute remains mute', () => {
  const controller = new audio.RecordingAudioController(async () => assert.fail('Default and receipt must not save implicitly'), () => {});
  controller.reconcile(audio.recordingAudioGrants());
  assert.equal(controller.view().mode, 'system');
  assert.throws(() => controller.selection(), /尚未就绪/);
  controller.accept({ revision: 0, sequence: 1, mode: 'system', grant: sound, editable: true });
  assert.deepEqual(controller.selection(), { revision: 0, mode: 'system', grant: sound });
  controller.accept({ revision: 2, sequence: 2, mode: 'mute', grant: null, editable: true });
  controller.reconcile(audio.recordingAudioGrants([plugin({ state: 'removed' })]));
  assert.deepEqual(controller.selection(), { revision: 2, mode: 'mute', grant: null });
});

test('native external nonmute receipt retains its actual mode and blocks actual App start with a reselect message, never silently becoming mute', async () => {
  for (const mode of ['microphone', 'both']) {
    const external = { revision: 8, sequence: 10, mode, grant: { pluginId: 'developer.audio', revision: 4, contributionId: 'capture-audio' }, editable: true };
    const original = structuredClone(external), events = [], errors = [];
    const controller = new audio.RecordingAudioController(undefined, () => {});
    controller.reconcile(audio.recordingAudioGrants()); controller.readSucceeded(external);
    assert.equal(controller.view().mode, mode); assert.equal(controller.view().error, '请重新选择声源');
    assert.deepEqual(controller.view().state, original); assert.throws(() => controller.selection(), /请重新选择声源/);
    controller.readFailed(Error('已有读取错误')); assert.equal(controller.view().error, '已有读取错误');
    controller.readSucceeded(external); assert.equal(controller.view().error, '请重新选择声源');
    const current = { id: 'scene', background: { id: 'background' }, regions: [{ id: 'region' }] };
    const start = await functionFrom('./App.tsx', 'startRecording', {
      scene: () => current, busy: () => false, recording: () => null, scroll: () => null, exitPreparing: () => false,
      recordingGrant: audio.recordingGrant, bridge: { native: true, startRecording: async () => assert.fail('Unselected external audio must not start') },
      showError: error => errors.push(error.message ?? error), recordingReady: () => true, setBusy: value => events.push(['busy', value]),
      spaceOperations: { begin: () => () => events.push('finish') }, videoPlayers: { pauseAll: () => events.push('pause') },
      recordingAudio: controller, flush: async () => assert.fail('Unselected audio must fail before flush'),
      disposed: false, sameAudioGrant: audio.sameAudioGrant, sameAudioSelection: audio.sameAudioSelection, clearReferenceDraft: () => assert.fail('No successful capture'),
    });
    await start('region');
    assert.deepEqual(errors, ['请重新选择声源']); assert.deepEqual(events, [['busy', true], 'pause', ['busy', false], 'finish']);
    assert.deepEqual(external, original); assert.deepEqual(controller.view().state, original);
    controller.readSucceeded({ revision: 9, sequence: 11, mode: 'mute', grant: null, editable: true });
    assert.deepEqual(controller.selection(), { revision: 9, mode: 'mute', grant: null });
  }
});

test('an explicit settings choice sends core audio and authorizes it only after the matching native receipt, without rewriting the old external state', async () => {
  const gate = deferred(), calls = [];
  const external = { revision: 8, sequence: 10, mode: 'microphone', grant: { pluginId: 'developer.audio', revision: 4, contributionId: 'capture-audio' }, editable: true };
  const controller = new audio.RecordingAudioController(async (...args) => { calls.push(args); return gate.promise; }, () => {});
  controller.reconcile(audio.recordingAudioGrants()); controller.readSucceeded(external);
  const choosing = controller.choose('both', sound); await turn();
  assert.deepEqual(calls, [[8, 'both', sound]]); assert.equal(controller.view().pending, true);
  assert.deepEqual(controller.view().state, external); assert.throws(() => controller.selection(), /尚未就绪/);
  const receipt = { revision: 9, sequence: 11, mode: 'both', grant: sound, editable: true };
  gate.resolve(receipt); await choosing;
  assert.deepEqual(controller.selection(), { revision: 9, mode: 'both', grant: sound });
  assert.equal(controller.view().error, ''); assert.equal(controller.view().draft, undefined);
  assert.deepEqual(external.grant, { pluginId: 'developer.audio', revision: 4, contributionId: 'capture-audio' });
});

test('actual bridge forwards precise base and audio grants without changing the selection', async () => {
  const calls = [], start = await functionFrom('./bridge.ts', 'startRecording', { native: true, invoke: async (...args) => calls.push(args) });
  const selected = { revision: 8, mode: 'both', grant: sound };
  await start('scene', 'region', selected, base);
  assert.deepEqual(calls, [['start_recording', { sceneId: 'scene', regionId: 'region', audio: selected, grant: base }]]);
});

test('actual App core start survives retired plugin changes and preserves delayed source/audio/exit fences', async () => {
  async function run(change = 'stable') {
    const gate = deferred(), events = [], errors = []; let records = [plugin()], exiting = false;
    let current = { id: 'scene', background: { id: 'background' }, regions: [{ id: 'region' }] };
    const selected = { revision: 8, mode: 'system', grant: sound }; let latestAudio = selected;
    const start = await functionFrom('./App.tsx', 'startRecording', {
      scene: () => current, busy: () => false, recording: () => null, scroll: () => null, exitPreparing: () => exiting,
      recordingGrant: () => audio.recordingGrant(records),
      bridge: { native: true, startRecording: async (...args) => events.push(['start', ...args]) },
      showError: value => errors.push(value.message ?? value), recordingReady: () => true, setBusy: value => events.push(['busy', value]),
      spaceOperations: { begin: () => () => events.push('finish') }, videoPlayers: { pauseAll: () => events.push('pause') },
      recordingAudio: { prepare: async () => selected, selection: () => latestAudio }, flush: () => { events.push('flush'); return gate.promise; },
      disposed: false, sameAudioGrant: audio.sameAudioGrant, sameAudioSelection: audio.sameAudioSelection, clearReferenceDraft: id => events.push(['clear', id]),
    });
    const pending = start('region'); await turn();
    assert.deepEqual(events.slice(0, 3), [['busy', true], 'pause', 'flush']);
    if (change === 'disabled') records = [plugin({ state: 'disabled', revision: 5 })];
    if (change === 'removed') records = [];
    if (change === 'source') current = { ...current, background: { id: 'replacement' } };
    if (change === 'scene') current = { ...current, id: 'other-scene' };
    if (change === 'closed') current = { ...current, closed: true };
    if (change === 'frozen') current = { ...current, frozen: true };
    if (change === 'exit') exiting = true;
    if (change === 'audio-revision') latestAudio = { ...selected, revision: 9 };
    if (change === 'audio-mode') latestAudio = { ...selected, mode: 'mute', grant: null };
    if (change === 'audio-grant') latestAudio = { ...selected, grant: { ...sound, revision: 2 } };
    gate.resolve(); await pending;
    assert.equal(events.at(-1), 'finish');
    assert.equal(events.filter(value => Array.isArray(value) && value[0] === 'busy' && value[1] === false).length, 1);
    return { events, errors };
  }
  for (const change of ['stable', 'disabled', 'removed']) {
    const result = await run(change);
    assert.deepEqual(result.events.find(value => value[0] === 'start'), ['start', 'scene', 'region', { revision: 8, mode: 'system', grant: sound }, base]);
    assert.deepEqual(result.errors, []);
    assert.equal(result.events.filter(value => Array.isArray(value) && value[0] === 'clear').length, 1);
  }
  for (const change of ['source', 'scene', 'closed', 'frozen', 'exit', 'audio-revision', 'audio-mode', 'audio-grant']) {
    const result = await run(change);
    assert.equal(result.events.some(value => value[0] === 'start'), false, change);
    assert.equal(result.events.some(value => value[0] === 'clear'), false, change);
    assert.match(result.errors[0], change.startsWith('audio-') ? /声音设置已变化/ : /录屏选区已变化/, change);
  }
});
