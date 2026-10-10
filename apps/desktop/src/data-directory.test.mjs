// Real settings client and Solid controller; synthetic receipts, no native/user files.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
const read = path => readFile(new URL(path, import.meta.url), 'utf8');
const compile = text => stripTypeScriptTypes(text, { mode: 'transform' });
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const contracts = new SourceTextModule(compile(await read('./settings-contracts.ts')));
await contracts.link(() => { throw new Error('Unexpected import'); }); await contracts.evaluate();
const clientModule = new SourceTextModule(compile(await read('./settings-client.ts')));
await clientModule.link(() => contracts); await clientModule.evaluate();
const { settingsClient } = clientModule.namespace;
const reactive = await import(new URL('../../../node_modules/solid-js/dist/solid.js', import.meta.url));
const source = await read('./components/SettingsFacts.tsx');
const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const node = ast.program.body.find(n => n.type === 'ExportNamedDeclaration' && n.declaration?.id?.name === 'createDataDirectorySettings');
assert.ok(node);
const controllerModule = new SourceTextModule(`import {createSignal,createEffect,on,onCleanup} from 'solid-js';\n${compile(source.slice(node.start, node.end))}`);
await controllerModule.link(() => synthetic(reactive)); await controllerModule.evaluate();
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const id = '623c814d-014a-4a29-bce1-780c3a9ee456';
const state = patch => ({ generation: 2, path: 'D:/synthetic/source', phase: 'committed', canChange: true, ...patch });
const proposal = patch => ({ proposalId: id, path: 'D:/synthetic/target', generation: 2, ...patch });
function mount(api, active) { let controller, dispose; reactive.createRoot(cleanup => { dispose = cleanup; controller = controllerModule.namespace.createDataDirectorySettings(api, active); }); return { controller, dispose }; }
const checks = [];
{
  assert.equal(contracts.namespace.validDataDirectoryState(state()), true);
  for (const patch of [{ generation: -1 }, { generation: 1.1 }, { path: '' }, { path: 'D:/bad\0path' }, { canChange: 1 }, { phase: 'finished' }]) assert.equal(contracts.namespace.validDataDirectoryState(state(patch)), false);
  for (const patch of [{ generation: -1 }, { proposalId: 'path' }, { proposalId: id.toUpperCase() }, { path: '' }]) assert.equal(contracts.namespace.validDataDirectoryProposal(proposal(patch)), false);
  const calls = [], api = settingsClient({ listen: async () => () => {}, invoke: async (command, args) => { calls.push({ command, args }); return command === 'get_data_directory_state' ? state() : command === 'choose_data_directory' ? proposal() : undefined; } });
  await api.dataDirectoryState(); const picked = await api.chooseDataDirectory(); await api.migrateDataDirectory(picked); await api.cancelDataDirectoryProposal(id);
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{ command: 'get_data_directory_state' }, { command: 'choose_data_directory' }, { command: 'migrate_data_directory', args: { proposalId: id, expectedGeneration: 2 } }, { command: 'cancel_data_directory_proposal', args: { proposalId: id } }]);
  checks.push('Native proposal and generation only; a renderer path never enters the migration request');
}
{
  let choices = 0, migrations = 0;
  const mounted = mount({ dataDirectoryState: async () => state({ canChange: false }), chooseDataDirectory: async () => { choices++; return proposal(); }, migrateDataDirectory: async () => migrations++ });
  await tick(); await mounted.controller.choose(); await mounted.controller.migrate(); assert.equal(choices, 0); assert.equal(migrations, 0); mounted.dispose();
  checks.push('A native busy/unavailable state prevents choosing or starting a migration');
}
{
  const choice = deferred(), cancellations = []; let choices = 0;
  const mounted = mount({ dataDirectoryState: async () => state(), chooseDataDirectory: () => { choices++; return choice.promise; }, cancelDataDirectoryProposal: async value => cancellations.push(value) });
  await tick(); const pending = mounted.controller.choose(); await mounted.controller.choose(); assert.equal(choices, 1); mounted.dispose(); choice.resolve(proposal()); await pending; assert.equal(mounted.controller.proposal(), undefined); assert.deepEqual(cancellations, [id]);
  checks.push('One real picker at a time; a late result after unmount releases its exact native proposal');
}
{
  const cancellations = [];
  const mounted = mount({ dataDirectoryState: async () => state(), chooseDataDirectory: async () => proposal({ generation: 1 }), cancelDataDirectoryProposal: async value => cancellations.push(value) });
  await tick(); await mounted.controller.choose(); assert.equal(mounted.controller.proposal(), undefined); assert.match(mounted.controller.error(), /已变化/); assert.deepEqual(cancellations, [id]); mounted.dispose();
  checks.push('A picker bound to an old generation is discarded and never offered for migration');
}
{
  let migrations = 0; const migration = deferred(); const cancellations = [];
  const mounted = mount({ dataDirectoryState: async () => state(), chooseDataDirectory: async () => proposal(), migrateDataDirectory: () => { migrations++; return migration.promise; }, cancelDataDirectoryProposal: async value => cancellations.push(value) });
  await tick(); await mounted.controller.choose(); const first = mounted.controller.migrate(); await mounted.controller.migrate(); assert.equal(migrations, 1); assert.equal(mounted.controller.state().path, 'D:/synthetic/source'); migration.reject(new Error('无法保存会话，迁移已取消')); await first; assert.equal(mounted.controller.proposal().proposalId, id); assert.match(mounted.controller.error(), /已取消/); assert.equal(mounted.controller.pending(), false); await mounted.controller.cancel(); assert.equal(mounted.controller.proposal(), undefined); assert.deepEqual(cancellations, [id]); mounted.dispose();
  checks.push('An aborted exit keeps the actual root and selection; no fake migration success or automatic retry');
}
{
  let migrate = 0;
  const mounted = mount({ dataDirectoryState: async () => state(), chooseDataDirectory: async () => null, migrateDataDirectory: async () => migrate++ });
  await tick(); await mounted.controller.choose(); await mounted.controller.migrate(); assert.equal(migrate, 0); assert.equal(mounted.controller.proposal(), undefined); assert.equal(mounted.controller.error(), ''); mounted.dispose();
  const bad = settingsClient({ listen: async () => () => {}, invoke: async () => undefined });
  await assert.rejects(bad.dataDirectoryState(), /无效/); await assert.rejects(bad.chooseDataDirectory(), /无效/);
  checks.push('Picker cancel is quiet; a malformed or missing host reply is never treated as cancel or success');
}
{
  const stopped = deferred(); let choices = 0;
  const mounted = mount({ dataDirectoryState: async () => state(), chooseDataDirectory: async () => { choices++; return proposal(); }, cancelDataDirectoryProposal: () => stopped.promise });
  await tick(); await mounted.controller.choose(); const replacing = mounted.controller.choose(); mounted.dispose(); stopped.resolve(); await replacing; assert.equal(choices, 1);
  checks.push('Unmount while replacing a selection cannot open another native picker after a slow cancel');
}
{
  let setActive, calls = 0;
  let data, dispose;
  reactive.createRoot(cleanup => { dispose = cleanup; const [value, set] = reactive.createSignal(false); setActive = set; data = controllerModule.namespace.createDataDirectorySettings({ dataDirectoryState: async () => { calls++; return state({ canChange: calls > 1 }); } }, value); });
  await tick(); assert.equal(calls, 0); setActive(true); await tick(); assert.equal(data.state().canChange, false); setActive(false); setActive(true); await tick(); assert.equal(data.state().canChange, true); assert.equal(calls, 2); dispose();
  checks.push('Entering the data tab refreshes native admission; a previous recording/busy state is not permanently latched');
}
{
  const commands = ['get-data-directory-state', 'choose-data-directory', 'cancel-data-directory-proposal', 'migrate-data-directory'];
  const settings = JSON.parse(await read('../src-tauri/capabilities/settings.json'));
  for (const command of commands) assert.ok(settings.permissions.includes(`allow-${command}`));
  for (const name of ['space', 'frozen-widget', 'pin', 'recording-controls', 'scroll']) {
    const capability = JSON.parse(await read(`../src-tauri/capabilities/${name}.json`));
    for (const command of commands) assert.ok(!capability.permissions.includes(`allow-${command}`));
  }
  checks.push('Directory inspection, picker, cancellation and migration are granted only to the actual settings capability');
}
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
