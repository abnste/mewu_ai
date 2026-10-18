// SPDX-License-Identifier: MPL-2.0
// Product contract/controllers and actual settings callbacks with synthetic IPC.
// No Windows registry, app process, user files, device or external request.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';
import test from 'node:test';
import { parse } from '@babel/parser';
const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const ts = source => stripTypeScriptTypes(source, { mode: 'transform' });
const modules = new Map();
async function load(file) {
  if (modules.has(file)) return modules.get(file);
  const module = new vm.SourceTextModule(ts(await raw(file))); modules.set(file, module);
  await module.link(name => load(`${name}.ts`)); await module.evaluate(); return module;
}
const { validSystemPreferences } = (await load('./settings-contracts.ts')).namespace;
const { settingsClient } = (await load('./settings-client.ts')).namespace;
const { SystemPreferencesController } = (await load('./system-preferences.ts')).namespace;
const state = extra => ({ version: 1, revision: 0, networkProxyMode: 'system', networkProxyUrl: '', launchAtStartup: false, startupRegistered: false, allowScreenShare: true, autoGenerateTitle: true, replaceSystemCaptureShortcut: true, ...extra });
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };

test('system contract refuses missing startup status, malformed modes and unsafe revision', () => {
  assert.equal(validSystemPreferences(state()), true);
  for (const patch of [{ revision: -1 }, { revision: Number.MAX_SAFE_INTEGER + 1 }, { networkProxyMode: 'unknown' }, { networkProxyUrl: '\0' }, { networkProxyUrl: 'x'.repeat(2049) }, { launchAtStartup: 'true' }, { startupRegistered: undefined }, { allowScreenShare: null }]) assert.equal(validSystemPreferences(state(patch)), false);
});
test('actual registered startup state is preserved when a different setting is edited', async () => {
  const calls = [], controller = new SystemPreferencesController(async (revision, value) => { calls.push({ revision, value }); return state({ revision: revision + 1, ...value, startupRegistered: value.launchAtStartup }); }, () => {});
  controller.loaded(state({ launchAtStartup: true, startupRegistered: false }));
  controller.edit({ networkProxyMode: 'direct' }); await controller.saveDraft();
  assert.equal(calls[0].value.launchAtStartup, false); assert.equal(controller.view().draft, undefined);
  controller.loaded(state({ revision: 2, launchAtStartup: false, startupRegistered: true }));
  controller.edit({ allowScreenShare: false }); await controller.saveDraft();
  assert.equal(calls[1].value.launchAtStartup, true);
});

test('title choice is strict, persisted through native save and survives editing unrelated settings', async () => {
  for (const autoGenerateTitle of [undefined, null, 'true', 1]) assert.equal(validSystemPreferences(state({ autoGenerateTitle })), false);
  const calls = [], controller = new SystemPreferencesController(async (revision, value) => {
    calls.push({ revision, value }); return state({ revision: revision + 1, ...value, startupRegistered: value.launchAtStartup });
  }, () => {});
  controller.loaded(state()); controller.edit({ autoGenerateTitle: false }); await controller.saveDraft();
  assert.equal(calls[0].value.autoGenerateTitle, false); assert.equal(controller.view().draft, undefined);
  controller.loaded(state({ revision: 1, autoGenerateTitle: false }));
  controller.edit({ networkProxyMode: 'direct' }); await controller.saveDraft();
  assert.equal(calls[1].value.autoGenerateTitle, false);
  const reopened = new SystemPreferencesController(async () => { throw Error('unexpected save'); }, () => {});
  reopened.loaded(controller.view().state); assert.equal(reopened.view().values.autoGenerateTitle, false);
  assert.equal(reopened.view().draft, undefined);
});
test('system capture choice remains strict, survives unrelated edits and rejects a mismatched native receipt', async () => {
  for (const replaceSystemCaptureShortcut of [undefined, null, 'false', 0]) assert.equal(validSystemPreferences(state({ replaceSystemCaptureShortcut })), false);
  const calls = [], controller = new SystemPreferencesController(async (revision, value) => {
    calls.push({ revision, value }); return state({ revision: revision + 1, ...value, startupRegistered: value.launchAtStartup });
  }, () => {});
  controller.loaded(state({ replaceSystemCaptureShortcut: false }));
  controller.edit({ networkProxyMode: 'direct' }); await controller.saveDraft();
  assert.equal(calls[0].value.replaceSystemCaptureShortcut, false);
  assert.equal(controller.view().draft, undefined);
  const refused = new SystemPreferencesController(async () => state({ revision: 1, replaceSystemCaptureShortcut: true }), () => {});
  refused.loaded(state()); refused.edit({ replaceSystemCaptureShortcut: false }); await refused.saveDraft();
  assert.equal(refused.view().state.replaceSystemCaptureShortcut, true);
  assert.equal(refused.view().draft.value.replaceSystemCaptureShortcut, false);
  assert.match(refused.view().error, /回执无效/);
});
test('system capture runtime warning is optional and strictly validated without changing the saved choice', () => {
  for (const systemCaptureShortcutWarning of [undefined, null, '', '无法接管系统截图快捷键']) {
    const value = state({ systemCaptureShortcutWarning });
    assert.equal(validSystemPreferences(value), true);
    assert.equal(value.replaceSystemCaptureShortcut, true);
  }
  for (const systemCaptureShortcutWarning of [false, 1, {}, [], 'x'.repeat(4097)]) assert.equal(validSystemPreferences(state({ systemCaptureShortcutWarning })), false);
});
test('capture and general pages cannot silently overwrite each other through an old full-settings draft', async () => {
  let saves = 0;
  const general = new SystemPreferencesController(async () => { saves++; throw Error('Unexpected stale save'); }, () => {});
  general.loaded(state()); general.edit({ networkProxyMode: 'direct' });
  general.accept(state({ revision: 1, replaceSystemCaptureShortcut: false }));
  await general.saveDraft();
  assert.equal(saves, 0); assert.equal(general.view().conflict, true);
  assert.equal(general.view().values.replaceSystemCaptureShortcut, true);
  general.loadLatest(state({ revision: 1, replaceSystemCaptureShortcut: false }), general.view().draft);
  general.edit({ autoGenerateTitle: false });
  assert.equal(general.view().draft.expectedRevision, 1);
  assert.equal(general.view().values.replaceSystemCaptureShortcut, false);
});
test('explicit startup enable repairs a missing Run item even when saved preference is already true', async () => {
  const calls = [], controller = new SystemPreferencesController(async (revision, value) => {
    calls.push({ revision, value }); return state({ revision, ...value, startupRegistered: value.launchAtStartup });
  }, () => {});
  controller.loaded(state({ revision: 4, launchAtStartup: true, startupRegistered: false }));
  controller.edit({ launchAtStartup: true });
  assert.equal(controller.view().draft.value.launchAtStartup, true);
  await controller.saveDraft();
  assert.equal(calls.length, 1); assert.equal(calls[0].revision, 4);
  assert.equal(controller.view().state.startupRegistered, true); assert.equal(controller.view().draft, undefined);
});
test('own save coalesces, preserves newer proxy draft and never rebases it on an unrelated event', async () => {
  const task = deferred(), calls = [], controller = new SystemPreferencesController((revision, value) => { calls.push({ revision, value }); return task.promise; }, () => {});
  controller.loaded(state()); controller.edit({ networkProxyMode: 'custom', networkProxyUrl: 'http://127.0.0.1:7890' });
  const save = controller.saveDraft(); assert.equal(controller.saveDraft(), save); await tick();
  controller.edit({ networkProxyUrl: 'http://127.0.0.1:7891' });
  controller.accept(state({ revision: 2, networkProxyMode: 'direct' }));
  task.resolve(state({ revision: 1, ...calls[0].value })); await save;
  assert.equal(calls.length, 1); assert.equal(controller.view().state.revision, 2);
  assert.equal(controller.view().draft.expectedRevision, 0); assert.equal(controller.view().draft.value.networkProxyUrl, 'http://127.0.0.1:7891'); assert.equal(controller.view().conflict, true);
});
test('failed or impossible native receipt retains the complete system draft', async () => {
  let invalid = false;
  const controller = new SystemPreferencesController(async () => { if (!invalid) throw Error('registry denied'); return state({ revision: 88 }); }, () => {});
  controller.loaded(state()); controller.edit({ launchAtStartup: true, allowScreenShare: false });
  await controller.saveDraft(); assert.equal(controller.view().state.launchAtStartup, false); assert.equal(controller.view().draft.value.launchAtStartup, true); assert.match(controller.view().error, /registry denied/);
  invalid = true; await controller.saveDraft(); assert.equal(controller.view().draft.value.allowScreenShare, false); assert.match(controller.view().error, /回执无效/);
});
test('system bridge listens before reads, fences stale state and sends only typed values', async () => {
  const task = deferred(), calls = [], seen = []; let receive, stops = 0;
  const client = settingsClient({ listen: async (name, callback) => { calls.push(name); receive = callback; return () => stops++; }, invoke: async (name, args) => { calls.push({ name, args }); return name === 'get_system_preferences' ? task.promise : state({ revision: 4, ...args, startupRegistered: args.launchAtStartup }); } });
  const opening = client.watchSystemPreferences(value => seen.push(value.revision)); await tick();
  receive(state({ revision: 3 })); task.resolve(state({ revision: 0 })); const stop = await opening;
  assert.deepEqual(seen, [3]); stop(); receive(state({ revision: 4 })); assert.equal(stops, 1); assert.deepEqual(seen, [3]);
  await client.saveSystemPreferences(3, { networkProxyMode: 'custom', networkProxyUrl: 'http://127.0.0.1:7890', launchAtStartup: true, allowScreenShare: false, autoGenerateTitle: false, replaceSystemCaptureShortcut: false });
  assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), { name: 'set_system_preferences', args: { expectedRevision: 3, networkProxyMode: 'custom', networkProxyUrl: 'http://127.0.0.1:7890', launchAtStartup: true, allowScreenShare: false, autoGenerateTitle: false, replaceSystemCaptureShortcut: false } });
});
test('late startup registry query cannot overwrite a same-revision noop save event', async () => {
  const task = deferred(), seen = []; let receive;
  const client = settingsClient({ listen: async (_name, callback) => { receive = callback; return () => {}; }, invoke: async () => task.promise });
  const opening = client.watchSystemPreferences(value => seen.push(value.startupRegistered)); await tick();
  receive(state({ startupRegistered: false })); task.resolve(state({ startupRegistered: true })); const stop = await opening;
  assert.deepEqual(seen, [false]); stop();
});

const source = await raw('./components/GeneralSettings.tsx'), ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const body = ast.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration.body.body;
const nodes = [];
function walk(node) { if (!node || typeof node !== 'object') return; if (node.type) nodes.push(node); for (const value of Object.values(node)) Array.isArray(value) ? value.forEach(walk) : walk(value); }
walk(ast);
const slice = node => source.slice(node.start, node.end);
function change(name) {
  const element = nodes.find(node => node.type === 'JSXOpeningElement' && node.attributes.some(attribute => ['label', 'aria-label'].includes(attribute.name?.name) && attribute.value?.expression && slice(attribute.value.expression) === `t('${name}')`));
  assert.ok(element, name); return slice(element.attributes.find(attribute => attribute.name?.name === 'onChange' || attribute.name?.name === 'onInput').value.expression);
}
test('actual general settings initialize real status, edit startup/sharing/proxy and save through native client', async () => {
  const calls = [], props = { active: false, preferences: { textSize: 'default', reduceMotion: false }, onPreferences(value) { props.preferences = value; } };
  props.api = { watchSystemPreferences: async receive => { receive(state()); return () => {}; }, getSystemPreferences: async () => state(), saveSystemPreferences: async (revision, value) => { calls.push({ revision, value }); return state({ revision: revision + 1, ...value, startupRegistered: value.launchAtStartup }); } };
  const context = vm.createContext({ props, SystemPreferencesController, createSignal: value => [() => value, next => { value = typeof next === 'function' ? next(value) : next; }], createEffect() {}, on() {}, onCleanup() {}, settingsApi: undefined });
  vm.runInContext(ts(`${body.filter(node => node.type !== 'ReturnStatement').map(slice).join('\n')}\nglobalThis.qa={initialize,view,controller,changes:{startup:${change('登录 Windows 后自动启动')},sharing:${change('允许屏幕共享看到框选和标注')},proxy:${change('网络代理')},labels:${change('显示按钮功能文字')},titles:${change('自动生成会话标题')}}};`), context);
  await context.qa.initialize();
  context.qa.changes.startup(true); context.qa.changes.sharing(false); context.qa.changes.proxy({ currentTarget: { value: 'direct' } }); context.qa.changes.labels(false);
  context.qa.changes.titles(false);
  await context.qa.controller.saveDraft();
  assert.equal(calls.length, 1); assert.equal(calls[0].value.launchAtStartup, true); assert.equal(calls[0].value.allowScreenShare, false); assert.equal(calls[0].value.networkProxyMode, 'direct'); assert.equal(props.preferences.showButtonLabels, false);
  assert.equal(context.qa.view().draft, undefined); assert.equal(context.qa.view().state.startupRegistered, true);
  assert.equal(calls[0].value.autoGenerateTitle, false); assert.equal(context.qa.view().values.autoGenerateTitle, false);
});

const reactive = await import(new URL('../../../node_modules/solid-js/dist/solid.js', import.meta.url));
const captureSource = await raw('./components/SystemCaptureShortcutSetting.tsx');
const captureAst = parse(captureSource, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const captureBody = captureAst.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration.body.body;
function captureShortcutSetting(api) {
  const context = vm.createContext({ props: { active: true, api }, SystemPreferencesController, ...reactive });
  let dispose;
  reactive.createRoot(cleanup => {
    dispose = cleanup;
    vm.runInContext(ts(`${captureBody.filter(node => node.type !== 'ReturnStatement').map(node => captureSource.slice(node.start, node.end)).join('\n')}\nglobalThis.qa={initialize,view,controller,change,checked,warning};`), context);
  });
  return { qa: context.qa, dispose };
}
test('actual screenshot settings toggle saves immediately through native CAS and retains an explicit failed choice', async () => {
  const calls = []; let fail = true;
  const setting = captureShortcutSetting({
    watchSystemPreferences: async receive => { receive(state({ revision: 4, networkProxyMode: 'direct', autoGenerateTitle: false })); return () => {}; },
    getSystemPreferences: async () => state(),
    saveSystemPreferences: async (revision, value) => { calls.push({ revision, value }); if (fail) throw Error('Hook unavailable'); return state({ revision: revision + 1, ...value }); },
  });
  await tick(); setting.qa.change(false); await tick();
  assert.equal(calls.length, 1); assert.equal(calls[0].revision, 4);
  assert.equal(calls[0].value.replaceSystemCaptureShortcut, false);
  assert.equal(calls[0].value.networkProxyMode, 'direct'); assert.equal(calls[0].value.autoGenerateTitle, false);
  assert.equal(setting.qa.view().state.replaceSystemCaptureShortcut, true);
  assert.equal(setting.qa.view().draft.value.replaceSystemCaptureShortcut, false);
  assert.match(setting.qa.view().error, /Hook unavailable/);
  fail = false; await setting.qa.controller.saveDraft();
  assert.equal(setting.qa.view().draft, undefined);
  assert.equal(setting.qa.view().state.replaceSystemCaptureShortcut, false);
  setting.dispose();
});
test('actual screenshot settings releases a late listener and cannot save after unmount', async () => {
  const ready = deferred(); let stopped = 0, saves = 0, receive;
  const setting = captureShortcutSetting({ watchSystemPreferences: callback => { receive = callback; return ready.promise; }, saveSystemPreferences: async () => { saves++; return state(); } });
  await tick(); setting.dispose(); receive(state()); ready.resolve(() => stopped++); await tick();
  setting.qa.change(false);
  assert.equal(stopped, 1); assert.equal(saves, 0); assert.equal(setting.qa.view().state, undefined);
});
test('actual screenshot settings shows an unavailable hook as off and allows repairing an already-enabled preference', async () => {
  let receive; const calls = [];
  const setting = captureShortcutSetting({
    watchSystemPreferences: async callback => { receive = callback; callback(state({ revision: 4, systemCaptureShortcutWarning: '无法接管系统截图快捷键' })); return () => {}; },
    saveSystemPreferences: async (revision, value) => { calls.push({ revision, value }); return state({ revision, ...value, systemCaptureShortcutWarning: null }); },
  });
  await tick();
  assert.equal(setting.qa.checked(), false);
  assert.equal(setting.qa.view().state.replaceSystemCaptureShortcut, true);
  assert.equal(setting.qa.warning(), '无法接管系统截图快捷键');
  setting.qa.change(true); await tick();
  assert.equal(calls.length, 1); assert.equal(calls[0].revision, 4);
  assert.equal(calls[0].value.replaceSystemCaptureShortcut, true);
  assert.equal('systemCaptureShortcutWarning' in calls[0].value, false);
  assert.equal(setting.qa.view().draft, undefined);
  assert.equal(setting.qa.checked(), true); assert.equal(setting.qa.warning(), '');
  receive(state({ revision: 4, systemCaptureShortcutWarning: '无法接管系统截图快捷键' }));
  assert.equal(setting.qa.checked(), false);
  setting.dispose();
});
