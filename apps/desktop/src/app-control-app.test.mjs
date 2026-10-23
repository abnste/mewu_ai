// SPDX-License-Identifier: MPL-2.0
// Execute actual App functions extracted by AST, with explicit native/UI ports.
// No native app, screenshot, user data or model request is opened by this suite.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';
import { webcrypto } from 'node:crypto';
const raw = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const ast = parse(raw, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const control = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL('./app-control.ts', import.meta.url), 'utf8'), { mode: 'transform' })).toString('base64')}`);
const submission = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL('./scene-submission.ts', import.meta.url), 'utf8'), { mode: 'transform' })).toString('base64')}`);
const { DrawingFlushRegistry } = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL('./drawing-flush.ts', import.meta.url), 'utf8'), { mode: 'transform' })).toString('base64')}`);
function productFunction(name) {
  let found;
  const walk = node => {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'FunctionDeclaration' && node.id?.name === name) found = node;
    for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(walk); else if (value && typeof value === 'object') walk(value);
  };
  walk(ast); assert.ok(found, name);
  return stripTypeScriptTypes(raw.slice(found.start, found.end), { mode: 'transform' });
}
function controlMount() {
  let found;
  const walk = node => {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'IfStatement' && node.test.type === 'LogicalExpression' && node.test.operator === '&&'
      && node.test.left.type === 'MemberExpression' && node.test.left.object.name === 'bridge' && node.test.left.property.name === 'native'
      && node.test.right.type === 'UnaryExpression' && node.test.right.argument.name === 'disposed') found = node;
    for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(walk); else if (value && typeof value === 'object') walk(value);
  };
  walk(ast); assert.ok(found);
  return stripTypeScriptTypes(`async function connectControl() { ${raw.slice(found.start, found.end)} }`, { mode: 'transform' });
}
const plain = value => JSON.parse(JSON.stringify(value));
function fixture() {
  const scene = { id: 'scene', background: { id: 'background' }, regions: [{ id: 'region', drawingRevision: 3 }], refs: [], draft: '', items: [], messages: [{ text: 'private-history-never-returned' }] };
  const f = { scene, saved: { revision: 8, activeSceneId: 'scene', scenes: [scene], agents: [{ instructions: 'private-soul' }] }, calls: [], busy: false, sending: false,
    selection: false, exiting: false, exporting: false, held: false, recording: null, scroll: null, operations: 0, references: {}, drafts: {}, errors: [] };
  const record = name => async (...args) => { f.calls.push([name, ...args]); f.saved.revision++; };
  const scope = {
    AppControlError: control.AppControlError, sameControlReferences: control.sameControlReferences, disposed: false, crypto: webcrypto, drawingFlush: new DrawingFlushRegistry(),
    objectFlush: { interacting: () => f.objectInteracting ?? false },
    scene: () => f.saved.scenes.find(value => value.id === f.saved.activeSceneId), snapshot: () => f.saved,
    busy: () => f.busy, sending: () => f.sending, selectionActive: () => f.selection, exportActive: () => f.exporting,
    exitPreparing: () => f.exiting, geometryView: () => ({ held: f.held }), recording: () => f.recording, scroll: () => f.scroll,
    setBusy: value => { f.busy = value; },
    spaceOperations: { begin() { f.operations++; let ended = false; return () => { assert.equal(ended, false); ended = true; f.operations--; }; } },
    flush: record('flush'), capture: record('capture'), addRegion: record('addRegion'), selectionCompleted: region => f.calls.push(['selectionCompleted', region]),
    setDraft: text => { f.drafts.scene = text; f.calls.push(['setDraft', text]); },
    setReferenceDrafts: fn => { f.references = fn(f.references); },
    referenceDrafts: () => f.references,
    command: record('command'), openBlackboard: record('openBlackboard'), finishBlackboard: record('finishBlackboard'), reopenBlackboard: record('reopenBlackboard'),
    isBlackboard: () => Boolean(f.board), setBlackboardEditing: record('setBlackboardEditing'),
    usesDrawingDocument: () => Boolean(f.rich), drawingCommand: record('drawingCommand'), drawingDocumentCommand: record('drawingDocumentCommand'),
    coreDrawingGrant: { pluginId: 'mewu.core.drawing', revision: 1, contributionId: 'drawing-tools' },
    startRecording: async (...args) => { f.calls.push(['startRecording', ...args]); return f.needsSelection ? 'selection-required' : 'started'; },
    startScroll: record('startScroll'), switchScene: record('switchScene'), newConversation: record('newConversation'), freeze: record('freeze'), closeScene: record('closeScene'),
    removeReferenceObject: record('removeReferenceObject'),
    cancel: record('cancel'),
    bridge: {
      getRecordingStatus: async () => f.nativeRecording ?? { id: 'recording', sceneId: 'scene', phase: 'starting' },
      controlRecording: record('controlRecording'), getScrollStatus: async () => f.nativeScroll ?? { id: 'scroll', sceneId: 'scene', phase: 'capturing', width: 200, height: 600, status: 'extended', preview: 'private-preview' },
      openSettings: record('openSettings'),
      importAssets: async (...args) => { f.calls.push(['importAssets', ...args]); f.scene.items.push({ id: 'imported-item', asset: { id: 'imported-asset', kind: 'image', name: 'input.png', path: 'private-source-path' } }); return f.saved; },
      exportControlImage: async (...args) => { f.calls.push(['exportControlImage', ...args]); return { path: 'D:/out.png', width: 100, height: 200 }; },
    },
    scrollBridge: { controlScrollCapture: record('controlScrollCapture') },
    clearReferenceDraft: sceneId => { delete f.references[sceneId]; }, accept: value => { f.saved = value; },
    preferences: () => ({ translationLanguage: 'zh-Hans' }),
    plugins: { runPluginOcr: async (...args) => { f.calls.push(['runPluginOcr', ...args]); f.scene.regions[0].ocr = { document: { lines: [{ text: 'recognized text' }] } }; return f.saved; } },
    translationBridge: { runTranslation: async input => { f.calls.push(['runTranslation', input]); f.scene.regions[0].translation = { overlay: { id: 'overlay' }, document: { targetLanguage: input.language, lines: [{ text: 'translated text' }] } }; return f.saved; } },
    pinBridge: { runPin: async input => { f.calls.push(['runPin', input]); return { id: 'pin' }; } },
    send: async (...args) => { f.calls.push(['send', ...args]); f.scene.run = { id: 'new-run', status: 'running' }; },
  };
  vm.createContext(scope);
  vm.runInContext(['withoutScene', 'assertControlIdle', 'flushControl', 'controlReceipt', 'withControlBusy', 'setControlReferences', 'executeControl'].map(productFunction).join('\n'), scope);
  return { f, scope };
}

test('actual App dispatcher routes the catalog into existing action handlers and returns compact receipts', async () => {
  const examples = [
    ['capture.open', {}, 'capture'],
    ['region.add', { region: { id: 'new' }, backgroundId: 'background' }, 'addRegion'],
    ['chat.setDraft', { text: 'draft' }, 'setDraft'],
    ['chat.send', { text: 'hello', refs: [] }, 'send'],
    ['chat.cancel', { expectedRunId: 'run' }, 'cancel'],
    ['blackboard.create', {}, 'openBlackboard'], ['blackboard.finish', {}, 'finishBlackboard'], ['blackboard.open', { itemId: 'board' }, 'reopenBlackboard'],
    ['drawing.apply', { command: { type: 'undo_drawing', sceneId: 'scene', regionId: 'region', backgroundId: 'background', expectedRevision: 3 } }, 'drawingCommand'],
    ['recording.start', { regionId: 'region' }, 'startRecording'], ['recording.control', { id: 'recording', action: 'stop' }, 'controlRecording'],
    ['scroll.start', { pluginId: 'scroll-plugin', revision: 2, contributionId: 'long', target: { sceneId: 'scene', regionId: 'region' } }, 'startScroll'],
    ['scroll.control', { id: 'scroll', action: 'stop' }, 'controlScrollCapture'],
    ['scene.activate', { sceneId: 'another' }, 'switchScene'], ['scene.new', {}, 'switchScene'], ['scene.newConversation', {}, 'newConversation'],
    ['scene.freeze', {}, 'freeze'], ['scene.close', {}, 'closeScene'],
    ['scene.command', { command: { type: 'rename_scene', sceneId: 'scene', title: 'name' } }, 'command'],
    ['references.set', { refs: [{ kind: 'region', id: 'region' }] }, 'command'], ['settings.open', {}, 'openSettings'],
    ['asset.import', { paths: ['D:/input.png'] }, 'importAssets'], ['image.export', { regionId: 'region', variant: 'original', path: 'D:/out.png' }, 'exportControlImage'],
    ['ocr.run', { pluginId: 'ocr', revision: 1, contributionId: 'text', target: { sceneId: 'scene', regionId: 'region' } }, 'runPluginOcr'],
    ['translation.run', { pluginId: 'translation', revision: 1, contributionId: 'text', target: { sceneId: 'scene', regionId: 'region' } }, 'runTranslation'],
    ['pin.create', { pluginId: 'pin', revision: 1, contributionId: 'image', target: { sceneId: 'scene', regionId: 'region' } }, 'runPin'],
  ];
  assert.equal(examples.length, control.appControlMethods.length);
  for (const [method, params, handler] of examples) {
    const { f, scope } = fixture(); const receipt = plain(await scope.executeControl(method, params));
    assert.ok(f.calls.some(call => call[0] === handler), `${method}: ${handler}`);
    assert.equal(receipt.sceneId, 'scene'); assert.equal(typeof receipt.revision, 'number');
    assert.equal(JSON.stringify(receipt).includes('private-'), false, method);
    assert.equal('scenes' in receipt, false); assert.equal('agents' in receipt, false);
    assert.equal(f.operations, 0); assert.equal(f.busy, false);
  }
});

test('actual strict shared handlers propagate rejection instead of pretending a skipped operation succeeded', async () => {
  for (const name of ['capture', 'switchScene', 'newConversation', 'freeze', 'closeScene', 'openBlackboard', 'finishBlackboard', 'reopenBlackboard', 'send', 'startRecording', 'startScroll', 'cancel']) {
    const { f, scope } = fixture(); f.busy = true;
    Object.assign(scope, { agentAvailable: () => true, agentReady: () => true, blackboardAvailable: () => true, capabilities: () => ({ blackboard: {} }),
      draft: () => 'text', refs: () => [], recordingGrant: () => ({}), recordingReady: () => true, scrollReady: () => true });
    vm.runInContext(productFunction(name), scope);
    const args = {
      capture: [true], switchScene: [{ type: 'new_scene' }, true], newConversation: [true], freeze: [true], closeScene: ['scene', true],
      openBlackboard: [true], finishBlackboard: [true], reopenBlackboard: ['board', true], send: [true], startRecording: ['region', true],
      startScroll: ['plugin', 1, 'long', { sceneId: 'scene', regionId: 'region' }, undefined, true], cancel: [true, 'old-run'],
    }[name];
    await assert.rejects(scope[name](...args), error => error instanceof control.AppControlError, name);
  }
});

test('actual flush adapter locks input for the shared barrier and releases operation tracking on failure', async () => {
  const { f, scope } = fixture();
  scope.flush = async () => { assert.equal(f.busy, true); assert.equal(f.operations, 1); throw Error('draft not saved'); };
  await assert.rejects(scope.flushControl(), /draft not saved/); assert.equal(f.busy, false); assert.equal(f.operations, 0);
  for (const field of ['busy', 'sending', 'selection', 'exporting', 'held']) {
    const next = fixture(); next.f[field] = true;
    await assert.rejects(next.scope.flushControl(), error => error.code === 'BUSY'); assert.equal(next.f.calls.length, 0);
  }
});

test('an idle mounted drawing owner allows blackboard finish and drawing writes through the actual control barrier', async () => {
  for (const method of ['blackboard.finish', 'drawing.apply']) {
    const { f, scope } = fixture(); f.selection = true; f.board = true;
    const flush = async () => {};
    flush.controlTarget = () => ({ sceneId: 'scene', backgroundId: 'background' });
    flush.interacting = () => false;
    scope.drawingFlush.register(flush);
    await scope.flushControl();
    const params = method === 'drawing.apply' ? { command: { type: 'undo_drawing', sceneId: 'scene', regionId: 'region', backgroundId: 'background', expectedRevision: 3 } } : {};
    const result = await scope.executeControl(method, params);
    assert.equal(result.sceneId, 'scene');
    assert.ok(f.calls.some(call => call[0] === (method === 'blackboard.finish' ? 'finishBlackboard' : 'drawingCommand')));
    assert.equal(f.selection, true, 'the existing UI editing-mode signal remains unchanged');
    assert.equal(f.busy, false); assert.equal(f.operations, 0);
  }
});

test('active, unknown or mismatched authoring owners cannot let control cancel or save human input', async () => {
  for (const state of ['active', 'unknown', 'wrong-scene', 'wrong-background', 'throw']) {
    const { f, scope } = fixture(); f.selection = true;
    const flush = async () => { throw Error('must not flush human input'); };
    flush.controlTarget = () => ({ sceneId: state === 'wrong-scene' ? 'other' : 'scene', backgroundId: state === 'wrong-background' ? 'other' : 'background' });
    if (state !== 'unknown') flush.interacting = () => { if (state === 'throw') throw Error('unavailable'); return state === 'active'; };
    scope.drawingFlush.register(flush);
    await assert.rejects(scope.flushControl(), error => error.code === 'BUSY', state);
    await assert.rejects(scope.executeControl('blackboard.finish', {}), error => error.code === 'BUSY', state);
    assert.deepEqual(f.calls, [], state); assert.equal(f.busy, false); assert.equal(f.operations, 0);
  }
  const object = fixture(); object.f.objectInteracting = true;
  await assert.rejects(object.scope.flushControl(), error => error.code === 'BUSY');
  assert.equal(object.f.selection, false, 'ordinary object movement has no persistent selection mode');
  assert.deepEqual(object.f.calls, []);
});

test('actual drawing route preserves core authority and selects the existing document path for rich objects', async () => {
  const { f, scope } = fixture(), command = { type: 'undo_drawing', sceneId: 'scene', regionId: 'region', backgroundId: 'background', expectedRevision: 3 };
  await scope.executeControl('drawing.apply', { command });
  assert.deepEqual(plain(f.calls[0]), ['drawingCommand', 'mewu.core.drawing', 1, 'drawing-tools', command]);
  const rich = fixture(); rich.f.rich = true;
  await rich.scope.executeControl('drawing.apply', { command }); assert.equal(rich.f.calls[0][0], 'drawingDocumentCommand');
  rich.scope.drawingDocumentCommand = async () => { throw Error('CAS rejected'); };
  await assert.rejects(rich.scope.executeControl('drawing.apply', { command }), /CAS rejected/); assert.equal(rich.f.operations, 0);
});

test('actual recording and scroll callbacks query native state and distinguish Portal selection from a started recording', async () => {
  const portal = fixture(); portal.f.needsSelection = true;
  const receipt = plain(await portal.scope.executeControl('recording.start', { regionId: 'region' }));
  assert.equal(receipt.requiresSelection, true); assert.equal(receipt.recordingStatus, null);
  const missing = fixture(); missing.scope.bridge.getRecordingStatus = async () => null;
  await assert.rejects(missing.scope.executeControl('recording.start', { regionId: 'region' }), error => error.code === 'OPERATION_FAILED');
  const stop = fixture(); stop.f.recording = { id: 'recording' };
  await stop.scope.executeControl('recording.control', { id: 'recording', action: 'stop' }); assert.equal(stop.f.calls[0][0], 'controlRecording');
  const scroll = fixture(); scroll.f.scroll = { id: 'scroll' };
  await scroll.scope.executeControl('scroll.control', { id: 'scroll', action: 'stop' });
  assert.deepEqual(plain(scroll.f.calls[0]), ['controlScrollCapture', 'scroll', 'finish']);
});

test('actual import export OCR translation and pin wait for native receipts and preserve source authority', async () => {
  const imported = fixture();
  const assets = plain(await imported.scope.executeControl('asset.import', { paths: ['D:/input.png'] }));
  assert.deepEqual(assets.items, [{ itemId: 'imported-item', assetId: 'imported-asset', kind: 'image', name: 'input.png' }]);
  const exported = fixture();
  const output = plain(await exported.scope.executeControl('image.export', { regionId: 'region', path: 'D:/out.png', variant: 'annotated' }));
  assert.equal(output.output.path, 'D:/out.png'); assert.deepEqual(plain(exported.f.calls[0]), ['exportControlImage', 'scene', 'region', 'D:/out.png', 'annotated']);
  const target = { sceneId: 'scene', regionId: 'region', backgroundId: 'background', drawingRevision: 3, x: 0, y: 0, width: 100, height: 200 };
  const recognized = fixture(); const ocr = plain(await recognized.scope.executeControl('ocr.run', { pluginId: 'ocr', revision: 4, contributionId: 'text', target }));
  assert.equal(ocr.text, 'recognized text'); assert.deepEqual(plain(recognized.f.calls[0].slice(2)), ['ocr', 4, 'text', target]);
  const translated = fixture(); const translation = plain(await translated.scope.executeControl('translation.run', { pluginId: 'translation', revision: 5, contributionId: 'translate', target }));
  assert.equal(translation.language, 'zh-Hans'); assert.equal(translation.overlayId, 'overlay');
  const pinned = fixture(); pinned.f.scene.regions[0].translation = { overlay: { id: 'existing-overlay' } };
  const pin = plain(await pinned.scope.executeControl('pin.create', { pluginId: 'pin', revision: 6, contributionId: 'pin-image', target, translationId: null }));
  assert.equal(pin.pinId, 'pin'); assert.equal(pinned.f.calls[0][1].translationId, null);
  for (const [method, port] of [['ocr.run', 'plugins'], ['translation.run', 'translationBridge'], ['pin.create', 'pinBridge']]) {
    const failed = fixture(), name = { 'ocr.run': 'runPluginOcr', 'translation.run': 'runTranslation', 'pin.create': 'runPin' }[method];
    failed.scope[port][name] = async () => { throw Error('grant revoked'); };
    await assert.rejects(failed.scope.executeControl(method, { pluginId: 'p', revision: 1, contributionId: 'c', target }), /grant revoked/);
    assert.equal(failed.f.operations, 0); assert.equal(failed.f.busy, false);
  }
});

test('actual reference setter rejects foreign targets and restores its own failed draft without overwriting newer UI edits', async () => {
  const foreign = fixture();
  await assert.rejects(foreign.scope.executeControl('references.set', { refs: [{ kind: 'item', id: 'missing' }] }), error => error.code === 'CONFLICT');
  assert.deepEqual(foreign.f.references, {}); assert.equal(foreign.f.calls.length, 0);
  const failed = fixture(); failed.scope.command = async () => { throw Error('native refs failed'); };
  await assert.rejects(failed.scope.executeControl('references.set', { refs: [{ kind: 'region', id: 'region' }] }), /native refs failed/);
  assert.deepEqual(plain(failed.f.references), {});
  const newer = fixture(); newer.scope.command = async () => { newer.f.references.scene = []; throw Error('native refs failed'); };
  await assert.rejects(newer.scope.executeControl('references.set', { refs: [{ kind: 'region', id: 'region' }] }), /native refs failed/);
  assert.deepEqual(newer.f.references.scene, []);
  const removed = fixture(); await removed.scope.executeControl('scene.command', { command: { type: 'remove_region', sceneId: 'scene', regionId: 'region' } });
  assert.deepEqual(plain(removed.f.calls[0]), ['removeReferenceObject', 'region', 'region', true]);
});

test('actual mount installs the space listener before native registration and binds claim completion to that renderer', async () => {
  const { f, scope } = fixture(); let actions, stops = 0;
  Object.assign(scope, {
    stopControlEvents: undefined, draft: () => 'draft', refs: () => [],
    attachAppControl: async value => { f.calls.push(['listen']); actions = value; return () => { stops++; }; },
  });
  Object.assign(scope.bridge, {
    native: true,
    registerControlRenderer: async () => { f.calls.push(['register']); return { rendererId: 'renderer' }; },
    claimControlRequest: async (...args) => { f.calls.push(['claim', ...args]); },
    completeControlRequest: async (...args) => { f.calls.push(['complete', ...args]); },
  });
  vm.runInContext(controlMount(), scope); await scope.connectControl();
  assert.deepEqual(plain(f.calls), [['listen'], ['register']]);
  await actions.claim({ id: 'op', nonce: 'secret' }); await actions.complete({ id: 'op', nonce: 'secret', result: {} });
  assert.deepEqual(plain(f.calls.slice(2)), [['claim', 'renderer', 'op', 'secret'], ['complete', 'renderer', { id: 'op', nonce: 'secret', result: {} }]]);
  scope.stopControlEvents(); assert.equal(stops, 1);
});

test('actual mount quietly detaches when connected to a native version without optional CLI commands', async () => {
  const { f, scope } = fixture(); let stops = 0;
  Object.assign(scope, { stopControlEvents: undefined, attachAppControl: async () => () => { stops++; } });
  scope.bridge.native = true; scope.bridge.registerControlRenderer = async () => { throw Error('unknown command'); };
  vm.runInContext(controlMount(), scope); await scope.connectControl();
  assert.equal(stops, 1); assert.equal(scope.stopControlEvents, undefined); assert.equal(f.calls.length, 0);
});

test('actual bridge wrappers use fixed native commands, exact renderer nonce fields and explicit file paths', async () => {
  const bridgeSource = await readFile(new URL('./bridge.ts', import.meta.url), 'utf8');
  const bridgeAst = parse(bridgeSource, { sourceType: 'module', plugins: ['typescript'] });
  const names = ['registerControlRenderer', 'claimControlRequest', 'completeControlRequest', 'getRecordingStatus', 'getScrollStatus', 'exportControlImage'];
  const selected = bridgeAst.program.body.filter(node => node.type === 'ExportNamedDeclaration' && names.includes(node.declaration?.id?.name));
  assert.equal(selected.length, names.length);
  const calls = [], scope = { native: true, invoke: async (command, args) => { calls.push([command, args]); return command === 'export_control_image' ? { path: 'D:/out.png' } : { rendererId: 'renderer' }; } };
  vm.createContext(scope); vm.runInContext(selected.map(node => stripTypeScriptTypes(bridgeSource.slice(node.declaration.start, node.declaration.end), { mode: 'transform' })).join('\n'), scope);
  await scope.registerControlRenderer(); await scope.claimControlRequest('renderer', 'id', 'nonce');
  await scope.completeControlRequest('renderer', { id: 'id', nonce: 'nonce', error: { code: 'CONFLICT', message: 'changed', executed: false } });
  await scope.getRecordingStatus(); await scope.getScrollStatus(); await scope.exportControlImage('scene', 'region', 'D:/out.png', 'original');
  assert.deepEqual(plain(calls), [
    ['register_control_renderer', null], ['claim_control_request', { rendererId: 'renderer', id: 'id', nonce: 'nonce' }],
    ['complete_control_request', { rendererId: 'renderer', id: 'id', nonce: 'nonce', error: { code: 'CONFLICT', message: 'changed', executed: false } }],
    ['get_recording_status', null], ['get_scroll_status', null], ['export_control_image', { sceneId: 'scene', regionId: 'region', path: 'D:/out.png', variant: 'original' }],
  ]);
  scope.native = false;
  await assert.rejects(scope.claimControlRequest('r', 'id', 'n'), /桌面/); assert.equal(calls.length, 6);
});

function sendFixture() {
  const { f, scope } = fixture(); f.scene.refs = [{ kind: 'region', id: 'region' }]; f.scene.run = { id: 'old', status: 'completed' }; f.saved.activeSceneId = 'scene';
  Object.assign(scope, {
    commands: Promise.resolve(), autosaveVersion: 0, debounce: undefined, exitFailure: undefined,
    agentReady: () => true, draft: () => f.drafts.scene ?? 'human unsent draft', refs: () => f.references.scene ?? f.scene.refs,
    visualAnnotationGrant: () => ({ pluginId: 'core', pluginRevision: 1, contributionId: 'annotate' }),
    annotationSendIdentity: () => 'source-identity', assertAnnotationSend: () => {}, hasAnnotationTarget: () => false,
    activeCaptureRegions: () => ({}), chatRegion: (_scene, refs, selected) => refs.some(ref => ref.kind === 'region' && ref.id === selected) ? selected : undefined,
    chatRegionIdentity: () => 'region-identity',
    voice: { cancel: async () => { f.calls.push(['voice.cancel']); } },
    flushVideo: async () => { f.calls.push(['video.flush']); }, flushDrawing: async () => { f.calls.push(['drawing.flush']); if (f.duringFlush) await f.duringFlush(); },
    regionGeometry: { flush: async () => { f.calls.push(['geometry.flush']); } },
    setSending: value => { f.sending = value; }, setSendErrors: () => {}, clearTimeout,
    setDrafts: fn => { f.drafts = fn(f.drafts); },
    submitSceneDraft: submission.submitSceneDraft,
    accept: next => { f.saved = next; },
  });
  scope.bridge.applyCommand = async value => { f.calls.push(['save', plain(value)]); return f.saved; };
  scope.bridge.sendMessage = async (...args) => { f.calls.push(['native.send', ...args]); if (f.sendFailure) throw Error('native send failed'); f.scene.run = { id: 'new-run', status: 'running' }; return f.saved; };
  vm.runInContext(productFunction('send'), scope); return { f, scope };
}
test('actual shared send submits explicit CLI text and references together without replacing an unsent human draft', async () => {
  const { f, scope } = sendFixture(), input = { text: 'external agent task', refs: [{ kind: 'region', id: 'region' }], expectedDraft: 'human unsent draft', regionId: 'region' };
  await scope.send(true, input);
  const saves = f.calls.filter(call => call[0] === 'save');
  assert.deepEqual(plain(saves.map(call => call[1])), [
    { type: 'set_draft', sceneId: 'scene', draft: 'external agent task' },
    { type: 'set_refs', sceneId: 'scene', refs: [{ kind: 'region', id: 'region' }] },
  ]);
  assert.equal(f.calls.at(-1)[0], 'native.send'); assert.equal(f.calls.at(-1)[3], 'region');
  assert.equal(f.sending, false); assert.equal(f.operations, 0);
});
test('actual shared send rejects a draft changed by the flush and propagates native send failure in strict mode', async () => {
  const changed = sendFixture(); changed.f.duringFlush = async () => { changed.f.drafts.scene = 'new human text'; };
  await assert.rejects(changed.scope.send(true, { text: 'external task', expectedDraft: 'human unsent draft' }), error => error.code === 'CONFLICT');
  assert.equal(changed.f.calls.some(call => call[0] === 'save'), false); assert.equal(changed.f.sending, false); assert.equal(changed.f.operations, 0);
  const failure = sendFixture(); failure.f.sendFailure = true;
  await assert.rejects(failure.scope.send(true, { text: 'external task' }), /native send failed/); assert.equal(failure.f.operations, 0);
});

test('actual shared send accepts equivalent reference fields in native JSON order and rejects changed references after flush', async () => {
  const { f, scope } = sendFixture();
  await scope.send(true, { text: 'external task', expectedRefs: [{ id: 'region', kind: 'region' }] });
  assert.equal(f.calls.filter(call => call[0] === 'native.send').length, 1);
  for (const references of [[], [{ id: 'other', kind: 'region' }], [{ id: 'region', kind: 'item' }]]) {
    const changed = sendFixture();
    changed.f.duringFlush = async () => { changed.f.references.scene = references; };
    await assert.rejects(changed.scope.send(true, { text: 'external task', expectedRefs: [{ id: 'region', kind: 'region' }] }), error => error.code === 'CONFLICT');
    assert.equal(changed.f.calls.some(call => call[0] === 'save' || call[0] === 'native.send'), false);
    assert.equal(changed.f.sending, false); assert.equal(changed.f.operations, 0);
  }
});
