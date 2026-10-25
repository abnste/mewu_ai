// SPDX-License-Identifier: MPL-2.0
// Actual component controller and Solid effects with synthetic IPC. No OS input,
// installed configuration, processes, models or network are touched by this test.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';
const source = await readFile(new URL('./components/HooksSettings.tsx', import.meta.url), 'utf8');
const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const component = ast.program.body.find(n => n.type === 'ExportDefaultDeclaration').declaration;
const returned = component.body.body.find(n => n.type === 'ReturnStatement');
const code = stripTypeScriptTypes(source.slice(0, returned.start) + 'return { arm, disarm, recording, error, add, edit, rule, ids, save, removeCapture, captureState, refresh, data, setTab }; }', { mode: 'transform' });
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const tick = () => new Promise(resolve => setImmediate(resolve));
const plain = value => JSON.parse(JSON.stringify(value));
async function fixture({ begin } = {}) {
  const ended = new Set(), opened = [], closed = [], hookLeases = [], writes = [], hooks = [];
  let view = { revision: 3, rules: [], runs: [], platform: 'windows', enabled: true }, stopped = 0;
  let capture = { revision: 5, sequence: 8, configured: { code: 'KeyS', alt: true, ctrl: false, shift: true } };
  const shortcut = {
    beginCaptureShortcutEdit: async id => { assert.equal(ended.has(id), false); opened.push(id); if (begin) await begin(id); },
    endCaptureShortcutEdit: async id => { ended.add(id); closed.push(id); },
    subscribeCaptureShortcut: async () => () => { stopped++; },
    getCaptureShortcut: async () => capture,
    setCaptureShortcut: async (revision, value) => { assert.equal(revision, capture.revision); writes.push(value); return capture = { ...capture, revision: revision + 1, sequence: capture.sequence + 1, configured: value }; },
  };
  const bridge = {
    editHookShortcut: async (id, active) => { hookLeases.push({ id, active }); },
    subscribeHooks: async cb => { hooks.push(cb); return () => { stopped++; }; },
    getHooks: async () => view,
    saveHook: async (revision, rule) => { assert.equal(revision, view.revision); view = { ...view, revision: revision + 1, rules: [{ ...plain(rule), revision: rule.revision + 1 }] }; return view; },
  };
  const contracts = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./hooks-contracts.ts', import.meta.url), 'utf8'), { mode: 'transform' }));
  await contracts.link(() => { throw Error('Unexpected dependency'); }); await contracts.evaluate();
  const editor = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./capture-shortcut-editor.ts', import.meta.url), 'utf8'), { mode: 'transform' }));
  await editor.link(() => { throw Error('Unexpected dependency'); }); await editor.evaluate();
  const mod = new SourceTextModule(code);
  await mod.link(name => {
    if (name === 'solid-js') return synthetic(solid);
    if (name.endsWith('/hooks-contracts')) return contracts;
    if (name.endsWith('/capture-shortcut-editor')) return editor;
    if (name.endsWith('/hooks-bridge')) return synthetic(bridge);
    if (name.endsWith('/shortcut-bridge')) return synthetic(shortcut);
    if (name.endsWith('/settings-bridge')) return synthetic({ settingsApi: {} });
    if (name.endsWith('/i18n')) return synthetic({ t: text => text });
    if (name === 'lucide-solid') return synthetic(Object.fromEntries(['Check', 'ChevronDown', 'ChevronRight', 'LoaderCircle', 'Play', 'Plus', 'Square', 'Trash2'].map(k => [k, () => {}])));
    if (name.endsWith('/SettingControls')) return synthetic({ Row() {}, SettingsAddButton() {}, Toggle() {} });
    if (name.endsWith('.css')) return synthetic({});
    return synthetic({ default() {} });
  }); await mod.evaluate();
  const original = globalThis.window;
  globalThis.window = { addEventListener() {}, removeEventListener() {} };
  let api, dispose, activate;
  solid.createRoot(cleanup => { dispose = cleanup; const [active, setActive] = solid.createSignal(true); activate = setActive; api = mod.namespace.default({ get active() { return active(); } }); });
  await tick(); await tick();
  return { api, opened, closed, hookLeases, writes, hooks, activate, setView: value => { view = value; }, stop: async () => { dispose(); await tick(); globalThis.window = original; }, stopped: () => stopped };
}
test('recording gets a fresh native lease on every focus and releases the exact old lease', async () => {
  const f = await fixture();
  try {
    await f.api.arm('rule-a'); assert.equal(f.api.recording(), 'rule-a');
    await f.api.disarm(); await f.api.arm('rule-b');
    assert.equal(f.api.recording(), 'rule-b'); assert.equal(f.opened.length, 2);
    assert.notEqual(f.opened[0], f.opened[1]); assert.deepEqual(f.closed, [f.opened[0]]);
    f.activate(false); await tick(); assert.equal(f.api.recording(), '');
    assert.deepEqual(f.closed, f.opened); assert.equal(f.api.error(), '');
  } finally { await f.stop(); }
});
test('a late OS editor receipt cannot reactivate a hidden or disposed Hooks page', async () => {
  let release; const pending = new Promise(resolve => { release = resolve; });
  const f = await fixture({ begin: () => pending });
  try {
    const flight = f.api.arm('rule-a'); await tick(); f.activate(false); release(); await flight;
    assert.equal(f.api.recording(), ''); assert.ok(f.closed.includes(f.opened[0]));
    assert.equal(f.hookLeases.filter(x => x.active).length, 0);
  } finally { await f.stop(); }
});
test('deleting the displayed capture binding updates the existing authoritative shortcut setting', async () => {
  const f = await fixture();
  try {
    await f.api.removeCapture(); assert.deepEqual(f.writes, [null]);
    assert.equal(f.api.captureState().configured, null); assert.equal(f.api.captureState().revision, 6);
  } finally { await f.stop(); }
});
test('saving preserves the rule identity and an older refresh cannot replace a newer view', async () => {
  const f = await fixture();
  try {
    f.api.setTab('prompt'); f.api.add();
    const [id] = f.api.ids(); f.api.edit(id, { name: 'Question', actions: [{ kind: 'injectPrompt', text: 'Use a brief answer.' }] });
    await f.api.save(id); assert.equal(f.api.error(), '');
    assert.equal(f.api.rule(id).revision, 1); assert.equal(f.api.rule(id).id, id);
    // The controller uses the real platform-specific factory and authoritative CAS.
    f.setView({ revision: 8, rules: [], runs: [], enabled: true, platform: 'windows' });
    await f.api.refresh();
    f.setView({ revision: 2, rules: [], runs: [], enabled: true, platform: 'windows' });
    await f.api.refresh(); assert.equal(f.api.data().revision, 8);
  } finally { await f.stop(); }
});
