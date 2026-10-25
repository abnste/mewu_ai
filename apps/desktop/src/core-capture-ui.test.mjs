// SPDX-License-Identifier: MPL-2.0
// Execute real component callbacks with synthetic regions/registry/IPC only.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';
import { parse } from '@babel/parser';
import test from 'node:test';
const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const plain = source => stripTypeScriptTypes(source, { mode: 'transform' });
const pure = async path => import(`data:text/javascript;base64,${Buffer.from(plain(await raw(path))).toString('base64')}`);
const core = await pure('./core-drawing.ts'), audio = await pure('./recording-audio.ts'), document = await pure('./drawing-document.ts');
const pluginCore = await pure('./plugin-core.ts'), categories = await pure('./plugin-categories.ts');
async function component(path) {
  const source = await raw(path), ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  const body = ast.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration.body.body;
  const functions = name => body.find(node => node.type === 'FunctionDeclaration' && node.id.name === name);
  const variable = name => body.find(node => node.type === 'VariableDeclaration' && node.declarations.some(value => value.id.name === name));
  const slice = node => source.slice(node.start, node.end);
  const all = [], walk = node => { if (!node || typeof node !== 'object') return; if (node.type) all.push(node); for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(walk); else if (value && typeof value === 'object') walk(value); };
  walk(ast);
  const jsx = name => all.find(node => node.type === 'JSXOpeningElement' && node.name.name === name);
  const attribute = (node, name) => node.attributes.find(value => value.name?.name === name)?.value?.expression;
  return { source, ast, body, functions, variable, slice, all, jsx, attribute };
}
const canvas = await component('./components/SpaceCanvas.tsx'), panel = await component('./components/BuiltinPlugins.tsx');
function run(source, values) { const context = vm.createContext(values); vm.runInContext(plain(source), context); return context; }
test('actual Canvas opens core drawing for an empty region or long image without plugin state; blocked/background fences remain', () => {
  const region = { id: 'region', drawings: [], imageOverride: { id: 'long' } }, calls = [];
  const scope = run(`${canvas.slice(canvas.functions('beginDrawing'))}\n${canvas.slice(canvas.variable('drawingTools'))}`, {
    coreDrawingTools: core.coreDrawingTools, props: { scene: { id: 'scene', background: { id: 'background' } }, plugins: [] },
    blocked: () => scope.denied, settledRegion: (region, callback) => callback(region),
    ocrRequests: { cancel() {} }, cancelGesture: undefined, hideToolbar() {}, setForceNew() {}, setActiveItemId() {}, setActive() {}, setSelectionActivity() {},
    setDrawingSession: value => calls.push(value), denied: false,
  });
  scope.beginDrawing(region);
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{ sceneId: 'scene', regionId: 'region', backgroundId: 'background', sourceId: 'long', kind: 'core' }]);
  assert.deepEqual(Array.from(vm.runInContext('drawingTools()', scope)), Array.from(core.coreDrawingTools));
  scope.denied = true; scope.beginDrawing(region); scope.denied = false; scope.props.scene.background = undefined; scope.beginDrawing(region);
  assert.equal(calls.length, 1);
});
test('actual Editor callback gives vector authoring exact core authority while saved rich/history changes use document access', async () => {
  const node = canvas.attribute(canvas.jsx('DrawingEditor'), 'onCommand'), calls = [];
  const rich = { id: 'rich', kind: 'rich' }, vector = { id: 'line', kind: 'line' };
  const region = { drawings: [rich, vector], drawingHistory: { undo: [{ before: null, after: vector }], redo: [] } };
  const scope = run(`const handler=${canvas.slice(node)}; globalThis.send=handler;`, {
    coreDrawingGrant: core.coreDrawingGrant, usesDrawingDocument: document.usesDrawingDocument,
    drawingSession: () => scope.session, session: { kind: 'core' }, region: () => region,
    props: { onDrawingCommand: (...args) => { calls.push(['core', ...args]); return Promise.resolve(); }, onDrawingDocumentCommand: command => { calls.push(['document', command]); return Promise.resolve(); } },
  });
  for (const command of [{ type: 'add_drawing', drawing: vector }, { type: 'update_drawing', drawing: vector }, { type: 'remove_drawing', drawingId: 'line' }]) {
    await scope.send(command); assert.deepEqual(calls.at(-1), ['core', 'mewu.core.drawing', 1, 'drawing-tools', command]);
  }
  for (const command of [{ type: 'update_drawing', drawing: rich }, { type: 'remove_drawing', drawingId: 'rich' }, { type: 'undo_drawing' }]) {
    await scope.send(command); assert.deepEqual(calls.at(-1), ['document', command]);
  }
  scope.session = undefined; await assert.rejects(scope.send({ type: 'add_drawing', drawing: vector }), /已切换/);
});
test('actual toolbar keeps drawing and recording with no plugins and reserves no sound control width', () => {
  const drawing = canvas.all.find(node => node.type === 'JSXOpeningElement' && node.name.name === 'button' && node.attributes.some(value => value.name?.name === 'class' && value.value?.value === 'capture-plugin') && node.attributes.some(value => value.name?.name === 'aria-label' && canvas.slice(value).includes('绘制')));
  const recording = canvas.all.find(node => node.type === 'JSXOpeningElement' && node.name.name === 'button' && node.attributes.some(value => value.name?.name === 'class' && value.value?.value === 'capture-record'));
  assert.ok(drawing); assert.ok(recording);
  const calls = [], scope = run(`${canvas.slice(canvas.variable('toolbarWidth'))}\nconst draw=${canvas.slice(canvas.attribute(drawing, 'onClick'))};const record=${canvas.slice(canvas.attribute(recording, 'onClick'))};globalThis.clicks=()=>{draw();record()};globalThis.width=toolbarWidth;`, {
    TOOLBAR_WIDTH: 405, pinEntries: () => [], workflowEntries: () => [], ocrEntries: () => [], scrollEntries: () => [], translationEntries: () => [], activeRegion: () => undefined,
    extensionButtonCount: () => scope.extensionCount, extensionCount: 0,
    region: () => ({ id: 'region' }), beginDrawing: value => calls.push(['draw', value.id]), props: { onRecordRegion: id => calls.push(['record', id]) },
  });
  scope.clicks(); assert.deepEqual(calls, [['draw', 'region'], ['record', 'region']]); assert.equal(scope.width(), 405); scope.props.showButtonLabels = false; assert.equal(scope.width(), 405);
  scope.extensionCount = 1; assert.equal(scope.width(), 457); scope.props.showButtonLabels = true; assert.equal(scope.width(), 457);
  scope.extensionCount = 0; assert.equal(scope.width(), 405);
  assert.equal(canvas.all.some(node => node.type === 'JSXOpeningElement' && node.name.name === 'RecordingAudioMenu'), false);
  assert.equal(canvas.ast.program.body.some(node => node.type === 'ImportDeclaration' && node.source.value.includes('RecordingAudioMenu')), false);
});
test('only actual settings select changes sound modes; App and Canvas expose receipt-only capture', async () => {
  const settings = await component('./components/RecordingAudioSettings.tsx'), app = await component('./App.tsx');
  const calls = [], coreGrant = audio.recordingAudioGrants()[0];
  let current = { revision: 0, sequence: 0, mode: 'system', grant: coreGrant, editable: true };
  const choose = settings.attribute(settings.jsx('select'), 'onChange');
  const scope = run(`${settings.body.filter(node => node.type !== 'ReturnStatement').map(settings.slice).join('\n')}\nconst select=${settings.slice(choose)};globalThis.choose=select;globalThis.initialize=initialize;globalThis.view=view;globalThis.controller=controller;`, {
    props: { active: false }, recordingAudioGrants: audio.recordingAudioGrants, RecordingAudioController: audio.RecordingAudioController,
    createSignal: value => [() => value, next => value = typeof next === 'function' ? next(value) : next], createEffect() {}, on() {}, onCleanup() {},
    subscribeRecordingAudio: async accept => { accept(current); return () => {}; }, getRecordingAudio: async () => current,
    setRecordingAudio: async (expectedRevision, mode, grant) => { calls.push({ expectedRevision, mode, grant }); return current = { revision: expectedRevision + 1, sequence: current.sequence + 1, mode, grant, editable: true }; },
  });
  assert.equal(scope.view().mode, 'system'); await scope.initialize();
  for (const mode of ['mute', 'system', 'microphone', 'both']) {
    scope.choose({ currentTarget: { value: mode } }); await new Promise(resolve => setImmediate(resolve));
    assert.equal(scope.controller.selection().mode, mode); assert.deepEqual(calls.at(-1).grant, mode === 'mute' ? null : coreGrant);
  }
  assert.equal(settings.ast.program.body.some(node => node.type === 'ImportDeclaration' && node.source.value.includes('plugin-bridge')), false);
  const audioController = app.variable('recordingAudio').declarations[0].init;
  assert.equal(audioController.type, 'NewExpression'); assert.equal(audioController.arguments[0].name, 'undefined');
  assert.equal(app.jsx('SpaceCanvas').attributes.some(value => value.name?.name === 'recordingAudio'), false);
});
test('builtin list hides retired core IDs, preserves user choices and cannot expose a retired community record', () => {
  const ids = ['mewu.annotations', 'mewu.drawing', 'mewu.recording', 'mewu.recording-audio', 'mewu.video-trim', 'mewu.gif', 'mewu.voice', 'mewu.ocr'];
  const records = ids.map((id, index) => ({ manifest: { id, name: id, description: '', contributions: [{ kind: index === ids.length - 1 ? 'selection.ocr' : 'input.speech-to-text' }] }, source:{type:'official'}, state: index % 2 ? 'disabled' : 'enabled', revision: index + 1 }));
  records.push({manifest:{id:'example.retired',name:'retired'},source:{type:'local'},state:'enabled',revision:1});
  const before = structuredClone(records), snapshot = { plugins: records }, catalog = records.slice(0,8).map(value => value.manifest);
  const scope = run(`${panel.slice(panel.variable('record'))}\n${panel.slice(panel.variable('entries'))}\nglobalThis.list=entries;`, {
    createMemo: callback => callback, props:{query:'',get filter(){return scope.filterValue}}, filterValue:'', catalog:()=>catalog,snapshot:()=>snapshot,
    isCoreReplacementPlugin:pluginCore.isCoreReplacementPlugin,pluginModuleTags:categories.pluginModuleTags,pluginTagLabels:categories.pluginTagLabels,t:value=>value,
  });
  assert.deepEqual(Array.from(scope.list(), value => value.id), ['mewu.voice','mewu.ocr']);
  scope.filterValue='audio'; assert.deepEqual(Array.from(scope.list(), value=>value.id), ['mewu.voice']);
  scope.filterValue='documents'; assert.deepEqual(Array.from(scope.list(), value=>value.id), ['mewu.ocr']);
  scope.filterValue='entertainment'; assert.deepEqual(Array.from(scope.list()), []);
  assert.equal(categories.pluginTags.length,16);assert.deepEqual(records,before);
});

test('installed community list excludes native, local and removed packages while retaining disabled GitHub plugins', async () => {
  const community = await component('./components/CommunityExtensions.tsx');
  const manifest = id => ({id,name:id,description:'',tags:['tools']});
  const records = [
    {manifest:manifest('github.enabled'),source:{type:'github',entry:{repository:'author/plugin'}},state:'enabled'},
    {manifest:manifest('github.disabled'),source:{type:'github',entry:{repository:'author/plugin'}},state:'disabled'},
    {manifest:manifest('github.removed'),source:{type:'github',entry:{repository:'author/plugin'}},state:'removed'},
    {manifest:manifest('native.module'),source:{type:'official'},state:'enabled'},
    {manifest:manifest('local.zip'),source:{type:'local',path:'fixture.zip'},state:'enabled'},
  ];
  const before=structuredClone(records), catalog=[{...manifest('market.entry'),repository:'author/market'}];
  const scope=run(`${community.slice(community.variable('record'))}\n${community.slice(community.variable('entries'))}\nglobalThis.list=entries;`,{
    createMemo:callback=>callback,props:{get pane(){return scope.paneValue},query:'',filter:''},paneValue:'installed',records:()=>records,catalog:()=>catalog,tagText:value=>value,
  });
  assert.deepEqual(Array.from(scope.list(),value=>value.id),['github.enabled','github.disabled']);
  scope.paneValue='discover';assert.deepEqual(Array.from(scope.list(),value=>value.id),['market.entry']);
  assert.deepEqual(records,before);
});

test('receipt-only capture controller defaults to computer audio but preserves explicitly saved mute and cannot save from capture', async () => {
  const controller = new audio.RecordingAudioController(undefined, () => {});
  controller.reconcile(audio.recordingAudioGrants()); assert.equal(controller.view().mode, 'system');
  controller.readSucceeded({ revision: 4, sequence: 4, mode: 'mute', grant: null, editable: true });
  assert.deepEqual(controller.selection(), { revision: 4, mode: 'mute', grant: null });
  await controller.choose('system', audio.recordingAudioGrants()[0]);
  assert.deepEqual(controller.selection(), { revision: 4, mode: 'mute', grant: null }); assert.equal(controller.view().draft, undefined);
  controller.readSucceeded({ revision: 5, sequence: 5, mode: 'system', grant: audio.recordingAudioGrants()[0], editable: true });
  assert.deepEqual(controller.selection(), { revision: 5, mode: 'system', grant: { pluginId: 'mewu.core.recording', revision: 1, contributionId: 'audio' } });
});
