// Synthetic contracts and actual Solid effects; no native host, files, devices or services.
// node --experimental-vm-modules apps/desktop/src/settings-ui.test.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import postcss from 'postcss';
const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const ts = text => stripTypeScriptTypes(text, { mode: 'transform' });
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const modules = new Map();
async function load(path) {
  if (modules.has(path)) return modules.get(path);
  const mod = new SourceTextModule(ts(await raw(path))); modules.set(path, mod);
  await mod.link(name => load(`./${name.slice(2)}.ts`)); await mod.evaluate(); return mod;
}
const contracts = (await load('./settings-contracts.ts')).namespace;
const { nativeSelectOwnsEscape } = (await load('./native-select-escape.ts')).namespace;
const { CapturePreferencesController } = (await load('./capture-preferences.ts')).namespace;
const { settingsClient } = (await load('./settings-client.ts')).namespace;
const checks = [], tick = () => new Promise(resolve => setImmediate(resolve));
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
const state = extra => ({ version: 1, revision: 4, captureDelaySeconds: 0, includeCursor: false, defaultImageFormat: 'png', ...extra });
const info = extra => ({ application: 'Mewu', version: '1.0.0-test', commit: null, channel: 'alpha', platform: 'windows', updateSupport: 'unconfigured', dataDirectory: 'D:/synthetic/.mewu', license: 'MPL-2.0', noticesAvailable: true, ...extra });
const normalize = value => JSON.parse(JSON.stringify(value));
{
  assert.equal(contracts.validCapturePreferences(state()), true);
  for (const patch of [{ revision: -1 }, { revision: 1.5 }, { version: 2 }, { captureDelaySeconds: 1 }, { captureDelaySeconds: '3' }, { includeCursor: 'false' }, { defaultImageFormat: 'gif' }]) assert.equal(contracts.validCapturePreferences(state(patch)), false);
  assert.equal(contracts.validSettingsInfo(info()), true);
  assert.equal(contracts.validSettingsInfo(info({ updateSupport: 'up_to_date' })), false);
  assert.equal(contracts.updateSupportLabel('unconfigured'), '尚未配置在线更新');
  assert.equal(contracts.validLicenseDocument({ title: '许可', text: '<script>literal</script>' }), true);
  checks.push('Native DTOs reject malformed values; unavailable updater is never called up-to-date; license is literal text');
}
{
  const calls = [], task = deferred(); let receive, stopped = 0;
  const client = settingsClient({ listen: async (name, callback) => { calls.push(name); receive = callback; return () => stopped++; }, invoke: async name => { calls.push(name); return task.promise; } });
  const values = [], opening = client.watchCapturePreferences(value => values.push(value.revision));
  await tick(); assert.deepEqual(calls, ['capture-preferences-changed', 'get_capture_preferences']);
  receive(state({ revision: 7 })); task.resolve(state({ revision: 4 })); const stop = await opening;
  assert.deepEqual(values, [7]); stop(); receive(state({ revision: 8 })); assert.deepEqual(values, [7]); assert.equal(stopped, 1);
  checks.push('Listener precedes query; delayed older get cannot overwrite an event; unsubscribe fences late delivery');
}
{
  let stops = 0;
  const client = settingsClient({ listen: async () => () => stops++, invoke: async () => { throw Error('read failed'); } });
  await assert.rejects(client.watchCapturePreferences(() => {}), /read failed/); assert.equal(stops, 1);
  const invalid = settingsClient({ listen: async () => () => {}, invoke: async () => ({ ok: true }) });
  await assert.rejects(invalid.getCapturePreferences(), /无效/); await assert.rejects(invalid.info(), /无效/); await assert.rejects(invalid.readLicenseDocument('license'), /无效/);
  checks.push('Failed initialization releases its listener; malformed host receipts remain errors, never synthetic defaults');
}
{
  const calls = [], client = settingsClient({ listen: async () => () => {}, invoke: async (name, args) => { calls.push({ name, args }); return name === 'set_capture_preferences' ? state({ revision: 5, ...args }) : name === 'settings_info' ? info() : name === 'read_license_document' ? { title: '许可', text: 'literal' } : undefined; } });
  await client.saveCapturePreferences(4, { captureDelaySeconds: 3, includeCursor: true, defaultImageFormat: 'jpeg' }); await client.info(); await client.openDataDirectory(); await client.readLicenseDocument('notices');
  assert.deepEqual(normalize(calls), [{ name: 'set_capture_preferences', args: { expectedRevision: 4, captureDelaySeconds: 3, includeCursor: true, defaultImageFormat: 'jpeg' } }, { name: 'settings_info' }, { name: 'open_data_directory' }, { name: 'read_license_document', args: { kind: 'notices' } }]);
  checks.push('RPC fields exactly match the fixed contract; directory open carries no renderer path and license read uses only a fixed kind');
}
{
  let invoked = 0, listened = 0;
  const mod = new SourceTextModule(ts(await raw('./settings-bridge.ts')));
  await mod.link(name => name.endsWith('/core') ? synthetic({ isTauri: () => false, invoke: async () => invoked++ }) : name.endsWith('/event') ? synthetic({ listen: async () => { listened++; return () => {}; } }) : synthetic({ settingsClient })); await mod.evaluate();
  await assert.rejects(mod.namespace.settingsApi.info(), /桌面版/); await assert.rejects(mod.namespace.settingsApi.openDataDirectory(), /桌面版/); await assert.rejects(mod.namespace.settingsApi.watchCapturePreferences(() => {}), /桌面版/);
  assert.equal(invoked, 0); assert.equal(listened, 0);
  checks.push('Actual bridge explicitly refuses ordinary browser host actions; it does not synthesize version, path, saved preferences or update success');
}
{
  const calls = [], listeners = [], mod = new SourceTextModule(ts(await raw('./settings-bridge.ts')));
  await mod.link(name => name.endsWith('/core') ? synthetic({ isTauri: () => true, invoke: async (command, args) => { calls.push({ command, args }); return state(); } }) : name.endsWith('/event') ? synthetic({ listen: async (name, callback, options) => { listeners.push({ name, callback, options }); return () => {}; } }) : synthetic({ settingsClient })); await mod.evaluate();
  const seen = [], stop = await mod.namespace.settingsApi.watchCapturePreferences(value => seen.push(value.revision));
  assert.deepEqual(listeners[0].options, { target: 'settings' });
  listeners[0].callback({ payload: state({ revision: 8 }) }); listeners[0].callback({ payload: { revision: 9 } }); stop();
  assert.deepEqual(seen, [4, 8]); assert.deepEqual(calls, [{ command: 'get_capture_preferences', args: undefined }]);
  checks.push('Native bridge scopes capture events to settings and rejects malformed delivered state');
}
{
  const task = deferred(), calls = [], controller = new CapturePreferencesController((revision, value) => { calls.push({ revision, value }); return task.promise; }, () => {});
  controller.accept(state()); controller.edit({ captureDelaySeconds: 3 }); const save = controller.saveDraft(); assert.equal(controller.saveDraft(), save);
  await tick(); controller.edit({ includeCursor: true }); task.resolve(state({ revision: 5, captureDelaySeconds: 3 })); await save;
  assert.equal(calls.length, 1); assert.equal(controller.view().draft.value.includeCursor, true); assert.equal(controller.view().draft.expectedRevision, 5);
  assert.equal(controller.view().state.includeCursor, false); assert.equal(controller.view().pending, false);
  checks.push('Concurrent Save coalesces; typing during a successful save survives and receives only the exact own-commit revision');
}
{
  const task = deferred(), controller = new CapturePreferencesController(() => task.promise, () => {});
  controller.accept(state()); controller.edit({ captureDelaySeconds: 3 }); const save = controller.saveDraft(); await tick();
  controller.accept(state({ revision: 6, defaultImageFormat: 'jpeg' })); controller.edit({ includeCursor: true }); task.resolve(state({ revision: 5, captureDelaySeconds: 3 })); await save;
  assert.equal(controller.view().state.revision, 6); assert.equal(controller.view().draft.expectedRevision, 4); assert.equal(controller.view().draft.value.includeCursor, true); assert.equal(controller.view().conflict, true);
  assert.match(controller.view().error, /载入最新/); controller.discard(); assert.equal(controller.view().values.defaultImageFormat, 'jpeg'); assert.equal(controller.view().draft, undefined);
  checks.push('External newer save wins over old ACK; draft is not rebased onto an unrelated revision and only explicit reload discards it');
}
{
  let fail = true, calls = 0;
  const controller = new CapturePreferencesController(async (revision, value) => { calls++; if (fail) throw Error('disk full'); return { version: 1, revision: revision + 1, ...value }; }, () => {});
  controller.accept(state()); controller.edit({ defaultImageFormat: 'jpeg' }); await controller.saveDraft();
  assert.equal(controller.view().state.defaultImageFormat, 'png'); assert.equal(controller.view().draft.value.defaultImageFormat, 'jpeg'); assert.match(controller.view().error, /disk full/);
  fail = false; await controller.saveDraft(); assert.equal(calls, 2); assert.equal(controller.view().draft, undefined); assert.equal(controller.view().state.defaultImageFormat, 'jpeg');
  checks.push('Save failure leaves effective native settings untouched and retains editable draft for one explicit retry');
}
{
  const task = deferred(), updates = [], controller = new CapturePreferencesController(() => task.promise, view => updates.push(view));
  controller.accept(state()); controller.edit({ includeCursor: true }); const save = controller.saveDraft(); await tick(); controller.dispose(); const count = updates.length;
  task.resolve(state({ revision: 5, includeCursor: true })); await save; controller.accept(state({ revision: 9 })); assert.equal(updates.length, count);
  const bad = new CapturePreferencesController(async () => state({ revision: 99 }), () => {}); bad.accept(state()); bad.edit({ includeCursor: true }); await bad.saveDraft(); assert.ok(bad.view().draft); assert.match(bad.view().error, /回执无效/);
  checks.push('Disposed controllers reject late UI updates; impossible save receipts never clear drafts');
}
{
  const controller = new CapturePreferencesController(async () => { throw Error('stale'); }, () => {});
  controller.accept(state()); controller.edit({ captureDelaySeconds: 3 }); const old = controller.view().draft;
  controller.edit({ includeCursor: true }); controller.loadLatest(state({ revision: 5, defaultImageFormat: 'jpeg' }), old);
  assert.equal(controller.view().draft.value.includeCursor, true); assert.equal(controller.view().conflict, true);
  controller.loadLatest(state({ revision: 5, defaultImageFormat: 'jpeg' }), controller.view().draft);
  assert.equal(controller.view().draft, undefined); assert.equal(controller.view().values.defaultImageFormat, 'jpeg');
  checks.push('Explicit reload fetches native state without clearing newer input typed while that read was pending');
}
{
  // Run the actual helper's client effects using Solid's browser reactive core.
  const reactive = await import(new URL('../../../node_modules/solid-js/dist/solid.js', import.meta.url));
  const text = await raw('./components/SettingsFacts.tsx');
  const ast = parse(text, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  const declaration = ast.program.body.find(node => node.type === 'ExportNamedDeclaration' && node.declaration?.id?.name === 'createSettingsInformation');
  assert.ok(declaration);
  const mod = new SourceTextModule(`import {createEffect,createSignal,on,onCleanup} from 'solid-js';\n${ts(text.slice(declaration.start, declaration.end))}`);
  await mod.link(() => synthetic(reactive)); await mod.evaluate();
  let calls = 0, dispose, setTab, data; const pending = deferred();
  reactive.createRoot(cleanup => { dispose = cleanup; const [tab, set] = reactive.createSignal('general'); setTab = set; data = mod.namespace.createSettingsInformation(() => tab() === 'about' || tab() === 'data', { info: () => { calls++; return pending.promise; } }); });
  await tick(); assert.equal(calls, 0); setTab('about'); await tick(); assert.equal(calls, 1); setTab('data'); setTab('capture'); setTab('about'); await tick(); assert.equal(calls, 1);
  dispose(); pending.resolve(info()); await tick(); assert.equal(data.info(), undefined);
  checks.push('Actual Solid effects load information once on first About/Data visit and ignore a receipt after unmount; tab changes do not create repeated reads');
}
{
  const controllerText = await readFile(new URL('./recording-audio.ts', import.meta.url), 'utf8');
  const mod = new SourceTextModule(ts(controllerText)); await mod.link(() => { throw Error('Unexpected runtime dependency'); }); await mod.evaluate();
  const grant = mod.namespace.recordingAudioGrants()[0]; let fail = true;
  assert.deepEqual(normalize(grant), { pluginId: 'mewu.core.recording', revision: 1, contributionId: 'audio' });
  const controller = new mod.namespace.RecordingAudioController(async revision => { if (fail) throw Error('录屏正在进行'); return { revision: revision + 1, sequence: 3, mode: 'both', grant, editable: true }; }, () => {});
  controller.reconcile([grant]); controller.accept({ revision: 1, sequence: 1, mode: 'mute', grant: null, editable: true });
  await controller.choose('both', grant); assert.throws(() => controller.selection(), /录屏正在进行/); assert.equal(controller.view().draft.mode, 'both');
  fail = false; await controller.choose('both', grant); assert.equal(controller.selection().mode, 'both');
  const retired = { manifest: { id: 'mewu.recording', contributions: [] }, state: 'removed', revision: 999 };
  controller.reconcile(mod.namespace.recordingAudioGrants([retired])); assert.equal(controller.selection().mode, 'both');
  controller.reconcile([]); assert.equal(controller.view().mode, 'both'); assert.throws(() => controller.selection(), /请重新选择声源/); controller.reconcile([{ ...grant, revision: 2 }]); assert.throws(() => controller.selection(), /请重新选择声源/);
  controller.reconcile(mod.namespace.recordingAudioGrants()); assert.equal(controller.selection().mode, 'both');
  controller.accept({ revision: 3, sequence: 4, mode: 'mute', grant: null, editable: true });
  controller.reconcile(mod.namespace.recordingAudioGrants()); assert.equal(controller.selection().mode, 'mute');
  checks.push('Settings uses core audio: save errors preserve draft, retired plugin state cannot disable it, unknown grant revision rejects and an explicit mute receipt stays mute');
}
{
  const dialog = await raw('./components/SettingsDialog.tsx'), capture = await raw('./components/CaptureSettings.tsx'), facts = await raw('./components/SettingsFacts.tsx');
  const parsed = parse(dialog, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  const labels = []; function visit(value) { if (!value || typeof value !== 'object') return; if (value.type === 'ObjectExpression') { const id = value.properties.find(p => p.key?.name === 'id')?.value; const label = value.properties.find(p => p.key?.name === 'label')?.value; const literal = label?.type === 'CallExpression' && label.callee?.name === 't' ? label.arguments[0] : label; if (literal?.type === 'StringLiteral' && id) labels.push(literal.value); } for (const child of Object.values(value)) { if (Array.isArray(child)) child.forEach(visit); else if (child && typeof child === 'object') visit(child); } } visit(parsed);
  assert.deepEqual(labels, ['通用', '截图与录屏', 'Agent 与连接', '插件', '数据', '关于']);
  assert.equal(dialog.includes('CaptureShortcutField'), false); assert.ok(capture.includes("CaptureShortcutField visible={props.active && pane() === 'screenshot'}")); assert.ok(capture.includes('选区外暗度')); assert.equal(dialog.includes('选区外暗度'), false);
  for (const forbidden of ['exportSnapshot', '导出会话', '导出 Agent', '本机</span>', 'Mewu 1.0</span>']) assert.equal(dialog.includes(forbidden), false);
  assert.equal(facts.includes('innerHTML'), false); assert.equal(facts.includes('检查更新'), false); assert.equal(facts.includes('更改目录'), false);
  checks.push('Actual dialog places hotkey and mask together under Capture; no static local/version row, JSON export, unimplemented updater/migration or unsafe license HTML');
}
{
  const source = await raw('./components/SettingsDialog.tsx'), ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  let handler;
  function visit(node) { if (!node || typeof node !== 'object') return; if (node.type === 'VariableDeclarator' && node.id?.name === 'keyboard') handler = node.init; for (const value of Object.values(node)) Array.isArray(value) ? value.forEach(visit) : visit(value); }
  visit(ast); assert.ok(handler);
  class Element { constructor(selector) { this.selector = selector; } closest(selector) { return selector === this.selector ? this : null; } }
  let closes = 0;
  const keyboard = new Function('Element', 'props', 'nativeSelectOwnsEscape', `${ts(`const keyboard = ${source.slice(handler.start, handler.end)};`)};return keyboard;`)(Element, { onClose: () => closes++ }, nativeSelectOwnsEscape);
  for (const target of [new Element('[data-settings-inner-dialog]'), new Element('.capture-shortcut-field input')]) keyboard({ key: 'Escape', target });
  keyboard({ key: 'Escape', defaultPrevented: true }); assert.equal(closes, 0);
  keyboard({ key: 'Escape', target: new Element(''), stopPropagation() {} }); assert.equal(closes, 1);
  const facts = await raw('./components/SettingsFacts.tsx');
  const compiled = transformSync(facts, { filename: 'SettingsFacts.tsx', parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'dom' }]], configFile: false, babelrc: false });
  assert.match(compiled.code, /addEventListener\([^\n]*"keydown", keydown\)/);
  checks.push('Nested license and shortcut Escape do not close settings; license keyboard is a real target listener, not a late delegated handler');
}
{
  const source = await raw('./App.tsx'), ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  let open, overlay, dialog;
  function visit(node, parent) {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'FunctionDeclaration' && node.id?.name === 'openConnection') open = node;
    if (node.type === 'JSXElement' && node.openingElement.name.name === 'SettingsDialog') { dialog = node; overlay = parent; }
    for (const value of Object.values(node)) Array.isArray(value) ? value.forEach(child => visit(child, node)) : visit(value, node);
  }
  visit(ast); assert.ok(open); assert.ok(dialog);
  assert.equal(overlay.type, 'JSXElement'); assert.equal(overlay.openingElement.name.name, 'div');
  const className = overlay.openingElement.attributes.find(attr => attr.name?.name === 'class')?.value?.value;
  assert.ok(className.split(/\s+/).includes('settings-surface')); assert.ok(className.split(/\s+/).includes('settings-surface-overlay'));
  const close = dialog.openingElement.attributes.find(attr => attr.name?.name === 'onClose')?.value.expression;
  let settings, nativeOpens = 0;
  const setSettings = next => { settings = next; };
  const bridge = { native: false, openSettings: async () => { nativeOpens++; } };
  // Execute the actual handlers with no location object: a route change would
  // fail this test instead of silently discarding the still-mounted space.
  const openConnection = new Function('bridge', 'setSettings', 'showError', `${ts(source.slice(open.start, open.end))};return openConnection;`)(bridge, setSettings, error => { throw error; });
  const closeSettings = new Function('setSettings', `${ts(`const closeSettings = (${source.slice(close.start, close.end)});`)};return closeSettings;`)(setSettings);
  await openConnection(); assert.equal(settings, 'agent'); assert.equal(nativeOpens, 0);
  closeSettings(); assert.equal(settings, undefined);
  bridge.native = true; await openConnection(); assert.equal(nativeOpens, 1); assert.equal(settings, undefined);
  assert.ok(ast.program.body.some(node => node.type === 'ImportDeclaration' && node.source.value === './settings-window.css'));
  checks.push('Actual browser open/close handlers use the shared full-surface wrapper without navigation; native open remains its existing window call');
}
{
  const css = postcss.parse(await raw('./settings-window.css'));
  const properties = selector => {
    const found = {};
    css.walkRules(rule => { if (rule.selectors.includes(selector)) rule.walkDecls(decl => { found[decl.prop] = decl.value; }); });
    return found;
  };
  const surface = properties('.settings-surface'), overlay = properties('.settings-surface-overlay');
  const backdrop = properties('.settings-surface .modal-backdrop'), dialog = properties('.settings-surface .settings-dialog');
  assert.equal(surface.padding, '0'); assert.equal(surface.background, '#f5f7fb');
  assert.equal(overlay.position, 'fixed'); assert.equal(overlay.inset, '0'); assert.equal(overlay['z-index'], '100');
  assert.equal(backdrop.padding, '0'); assert.equal(backdrop.background, 'transparent'); assert.equal(backdrop.height, '100%');
  assert.equal(dialog.width, '100%'); assert.equal(dialog.height, '100%'); assert.equal(dialog['max-height'], 'none'); assert.equal(dialog['box-shadow'], 'none');
  checks.push('Shared parsed CSS overrides the old 24px dark modal frame with full opaque viewport, full dialog height and no outer shadow (not a browser pixel test)');
}
console.log(JSON.stringify({ passed: checks.length, checks, boundaries: 'Synthetic tests only; no Cargo, browser, native IPC, user data, device or service used.' }, null, 2));
