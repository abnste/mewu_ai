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
const code = stripTypeScriptTypes(source.slice(0, returned.start) + 'return { arm, disarm, recording, error, add, edit, rule, ids, save, remove, refresh, data, setTab }; }', { mode: 'transform' });
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const tick = () => new Promise(resolve => setImmediate(resolve));
const plain = value => JSON.parse(JSON.stringify(value));
async function fixture({ begin } = {}) {
  const ended = new Set(), opened = [], closed = [], hookLeases = [], writes = [], hooks = [];
  let view = { revision: 3, rules: [], runs: [], platform: 'windows', enabled: true }, stopped = 0, shortcutEvents, recordEvent, blurred = 0;
  let capture = { revision: 5, sequence: 8, configured: { code: 'KeyS', alt: true, ctrl: false, shift: true } };
  const shortcut = {
    beginCaptureShortcutEdit: async id => { assert.equal(ended.has(id), false); opened.push(id); if (begin) await begin(id); },
    endCaptureShortcutEdit: async id => { ended.add(id); closed.push(id); },
    subscribeCaptureShortcut: async callbacks => { shortcutEvents = callbacks; return () => { stopped++; }; },
    getCaptureShortcut: async () => capture,
    setCaptureShortcut: async (revision, value) => { assert.equal(revision, capture.revision); writes.push(value); return capture = { ...capture, revision: revision + 1, sequence: capture.sequence + 1, configured: value }; },
  };
  const bridge = {
    editHookShortcut: async (id, active) => { hookLeases.push({ id, active }); },
    subscribeHooks: async (cb, recorded) => { hooks.push(cb); recordEvent=recorded; return () => { stopped++; }; },
    getHooks: async () => view,
    saveHook: async (revision, rule) => { assert.equal(revision, view.revision); view = { ...view, revision: revision + 1, rules: [{ ...plain(rule), revision: rule.revision + 1 }] }; return view; },
    removeHook: async (revision,id,ruleRevision)=>{assert.equal(revision,view.revision);assert.equal(view.rules.find(r=>r.id===id)?.revision,ruleRevision);writes.push(id);view={...view,revision:revision+1,rules:view.rules.filter(r=>r.id!==id)};return view;},
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
  const originalDocument = globalThis.document;
  globalThis.window = { addEventListener() {}, removeEventListener() {} };
  globalThis.document = { activeElement: { matches: selector => selector === '[data-hook-shortcut]', blur: () => { blurred++; globalThis.document.activeElement=null; } } };
  let api, dispose, activate;
  solid.createRoot(cleanup => { dispose = cleanup; const [active, setActive] = solid.createSignal(true); activate = setActive; api = mod.namespace.default({ get active() { return active(); } }); });
  await tick(); await tick();
  return { api, opened, closed, hookLeases, writes, hooks, activate, record: (id,key)=>recordEvent(id,key), endRecord: id=>shortcutEvents.ended(id), blurred:()=>blurred, setView: value => { view = value; }, stop: async () => { dispose(); await tick(); globalThis.window = original; globalThis.document=originalDocument; }, stopped: () => stopped };
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
test('native recording receipts and expiry can only affect their current editor lease', async () => {
  const f = await fixture();
  try {
    f.api.setTab('shortcut'); f.api.add(); const [first] = f.api.ids();
    await f.api.arm(first); const old=f.opened.at(-1);
    f.api.add(); const second=f.api.ids().find(id=>id!==first); await f.api.arm(second); const current=f.opened.at(-1);
    f.record(old,'F9'); assert.equal(f.api.rule(second).trigger.accelerator,'');
    f.record(current,'F10'); assert.equal(f.api.rule(second).trigger.accelerator,'F10');
    f.endRecord(old); assert.equal(f.api.recording(),second); assert.equal(f.blurred(),0);
    f.endRecord(current); await tick(); assert.equal(f.api.recording(),''); assert.equal(f.blurred(),1);
  } finally { await f.stop(); }
});
test('a migrated capture binding uses the same editable rule and removal CAS as every shortcut', async () => {
  const f = await fixture();
  try {
    f.api.setTab('shortcut');f.api.add();const [id]=f.api.ids();
    f.api.edit(id,{name:'唤起截图',trigger:{kind:'shortcut',accelerator:'F8'},actions:[{kind:'app',method:'capture.open',params:{}}]});
    await f.api.save(id);assert.equal(f.api.rule(id).actions[0].method,'capture.open');
    f.api.edit(id,{name:'打开设置',actions:[{kind:'app',method:'settings.open',params:{}}]});await f.api.save(id);
    assert.equal(f.api.rule(id).actions[0].method,'settings.open');
    await f.api.remove(id);assert.deepEqual(f.writes,[id]);assert.equal(f.api.ids().length,0);
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

test('reentering Hooks reloads changed MCP metadata without losing an unsaved rule', async () => {
  const f = await fixture();
  try {
    f.api.setTab('event'); f.api.add(); const [id] = f.api.ids();
    f.api.edit(id, { name: 'Unsaved local rule' });
    f.activate(false); await tick();
    const servers = [{ id: 'test-mcp', name: 'Newly enabled MCP', tools: [{ name: 'echo' }] }];
    f.setView({ revision: 3, rules: [], runs: [], enabled: true, platform: 'windows', mcpServers: servers });
    f.activate(true); await tick(); await tick();
    assert.deepEqual(plain(f.api.data().mcpServers), servers);
    assert.equal(f.api.rule(id).name, 'Unsaved local rule');
  } finally { await f.stop(); }
});
