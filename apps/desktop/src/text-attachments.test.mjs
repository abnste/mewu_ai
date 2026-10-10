// node --experimental-vm-modules apps/desktop/src/text-attachments.test.mjs
// Synthetic File bytes and IPC only; no user files, dialogs or clipboard.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule, createContext } from 'node:vm';
import { webcrypto } from 'node:crypto';
import { loadProviderPresets, loadAssetImport, loadGeometryHistory } from './geometry-test-module.mjs';
const source = async file => stripTypeScriptTypes(await readFile(new URL(file, import.meta.url), 'utf8'), { mode: 'transform' });
const plain = value => JSON.parse(JSON.stringify(value));
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const synthetic = (values, context) => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, context ? { context } : undefined);
const policy = (await loadAssetImport()).namespace;
const checks = [];
const buffer = value => new TextEncoder().encode(value).buffer;
const file = (name, text) => new File([text], name);

for (const name of ['note.txt', 'NOTE.MD', '配置.json', '表格.csv']) assert.equal(policy.importedKind(name), 'text');
assert.equal(policy.importedKind('demo.html'), 'html'); assert.equal(policy.importedKind('image.svg'), 'svg');
assert.throws(() => policy.importedKind('report.pdf'), /不支持/);
assert.equal(policy.decodeImportedText(buffer('\uFEFF中文\n<script>literal</script>'), 'text'), '中文\n<script>literal</script>');
assert.throws(() => policy.decodeImportedText(new Uint8Array([0xc3, 0x28]).buffer, 'text'), /UTF-8/);
assert.throws(() => policy.decodeImportedText(buffer('a\0b'), 'text'), /NUL/);
assert.equal(policy.decodeImportedText(buffer('a\0b'), 'html'), 'a\0b');
checks.push('Four original text extensions, strict UTF-8/BOM and literal markup; only Text rejects NUL');

const long = '中'.repeat(21_845) + '😀\r\n' + 'x'.repeat(70_000), boundaries = policy.textPageBoundaries(long);
let reconstructed = '';
for (let index = 0; index + 1 < boundaries.length; index++) {
  const page = long.slice(boundaries[index], boundaries[index + 1]); reconstructed += page;
  assert.ok(new TextEncoder().encode(page).length <= 65_536);
  assert.ok(!/[\uD800-\uDBFF]$/.test(page)); assert.ok(!/^[\uDC00-\uDFFF]/.test(page));
}
assert.equal(reconstructed, long); assert.deepEqual(plain(policy.textPageBoundaries('')), [0, 0]);
checks.push('64 KiB pages are byte bounded, preserve whitespace/Unicode and reconstruct the complete file');

let serial = 0; const revoked = [], created = [];
const env = { id: () => `id-${++serial}`, imageSize: async () => ({ width: 100, height: 80 }), createUrl: f => { const url = `blob:${f.name}:${serial}`; created.push(url); return url; }, revokeUrl: url => revoked.push(url) };
await assert.rejects(policy.prepareImportFiles([file('ok.txt', 'hello'), file('bad.json', new Uint8Array([255]))], env), /UTF-8/);
assert.deepEqual(revoked, created);
await assert.rejects(policy.prepareImportFiles(Array.from({ length: 13 }, () => file('a.txt', 'x')), env), /12/);
await assert.rejects(policy.prepareImportFiles([{ name: 'a.txt', size: 2 * 1024 * 1024 + 1 }], env), /2 MiB/);
await assert.rejects(policy.prepareImportFiles(Array.from({ length: 5 }, () => ({ name: 'a.png', size: 32 * 1024 * 1024 })), env), /128 MiB/);
await assert.rejects(policy.prepareImportFiles([file('a.png', 'x')], { ...env, imageSize: async () => ({ width: 10_000, height: 4_001 }) }), /尺寸/);
const boundary = await policy.prepareImportFiles([file('large.txt', 'x'.repeat(2 * 1024 * 1024))], env); assert.equal(boundary[0].text.length, 2 * 1024 * 1024);
const empty = await policy.prepareImportFiles([file('empty.txt', ''), file('empty.html', ''), file('empty.svg', '')], env); assert.ok(empty.every(value => value.text === ''));
await assert.rejects(policy.prepareImportFiles([file('empty.png', '')], env), /图片/);
checks.push('Late validation failure revokes the complete staged batch; 12 files/2 MiB text/128 MiB batch/40M pixels enforced');

async function bridgeFixture(native = false) {
  const calls = [], inputs = [], urls = new Map(), removed = [], downloads = [];
  let pickerFiles = [];
  const context = createContext({ crypto: webcrypto, structuredClone, Date, Map, Set, Blob, File, setTimeout, clearTimeout, TextEncoder, TextDecoder,
    URL: { createObjectURL: value => { const path = `blob:fixture-${urls.size + removed.length}`; urls.set(path, value); return path; }, revokeObjectURL: value => { removed.push(value); urls.delete(value); } },
    createImageBitmap: async () => ({ width: 100, height: 80, close() {} }),
    document: { body: { appendChild() {} }, createElement: tag => tag === 'input' ? (() => { const input = { files: pickerFiles, remove() {}, addEventListener() {}, click() { queueMicrotask(() => input.onchange()); } }; inputs.push(input); return input; })() : { remove() {}, click() { downloads.push({ href: this.href, download: this.download }); } } },
  });
  const snapshot = { schemaVersion: 1, activeSceneId: 'native-scene', scenes: [], agents: [], connections: [] };
  const core = synthetic({ isTauri: () => native, invoke: async (command, args) => { calls.push({ command, args }); if (command === 'import_assets') return snapshot; if (command === 'read_artifact') return 'native text'; } }, context);
  const importedAssets = await loadAssetImport(context), history = await loadGeometryHistory(context);
  const connection = new SourceTextModule(await source('connection-policy.ts'), { context }); await connection.link(() => { throw Error('Unexpected'); });
  const mosaic = new SourceTextModule(await source('mosaic-preview.ts'), { context }); await mosaic.link(() => { throw Error('Unexpected'); });
  const bridge = new SourceTextModule(await source('bridge.ts'), { context, initializeImportMeta: meta => { meta.glob = () => ({}); } });
  await bridge.link(async name => name === './provider-presets' ? await loadProviderPresets(context) : name === './asset-import' ? importedAssets : name === './region-geometry-history' ? history : name === './connection-policy' ? connection : name === './mosaic-preview' ? mosaic : name.endsWith('/core') ? core : synthetic({ listen: async () => () => {} }, context));
  await bridge.evaluate(); return { api: bridge.namespace, calls, inputs, urls, removed, downloads, pick: values => { pickerFiles = values; } };
}
const preview = await bridgeFixture();
let state = await preview.api.getSnapshot(), sceneId = state.activeSceneId;
preview.pick([file('原名.txt', '\uFEFF第一行\n<script>纯文本</script>'), file('data.csv', 'x,y\n1,2')]);
state = await preview.api.importAssets(sceneId);
let scene = state.scenes.find(value => value.id === sceneId);
assert.equal(scene.items.length, 2); assert.deepEqual(plain(scene.refs), scene.items.map(item => ({ kind: 'item', id: item.id })));
assert.equal(await preview.api.readArtifact(scene.items[0].asset.id), '第一行\n<script>纯文本</script>');
await preview.api.exportTextAsset(scene.items[0].asset.id); assert.equal(preview.downloads[0].download, '原名.txt');
assert.deepEqual(new Uint8Array(await preview.urls.get(preview.downloads[0].href).arrayBuffer()).slice(0, 3), new Uint8Array([239, 187, 191]));
checks.push('Real preview File import atomically adds items and refs; native filename and original BOM bytes survive export');

preview.pick([file('valid.md', 'good'), file('invalid.json', new Uint8Array([255]))]);
await assert.rejects(preview.api.importAssets(sceneId), /UTF-8/);
let unchanged = (await preview.api.getSnapshot()).scenes.find(value => value.id === sceneId);
assert.deepEqual(plain(unchanged), plain(scene)); assert.equal(preview.urls.size, 2);
const waiting = deferred(); preview.pick([{ name: 'late.txt', size: 2, arrayBuffer: () => waiting.promise }]);
const pending = preview.api.importAssets(sceneId); await tick();
await preview.api.applyCommand({ type: 'new_scene' }); waiting.resolve(buffer('ok'));
await assert.rejects(pending, /会话已变化/); assert.equal(preview.urls.size, 2);
state = await preview.api.getSnapshot(); assert.equal(state.scenes.find(value => value.id === state.activeSceneId).items.length, 0);
assert.equal(state.scenes.find(value => value.id === sceneId).items.length, 2);
checks.push('Failure or scene change during async validation cannot partially publish or leak staged URLs into another scene');

const native = await bridgeFixture(true);
await native.api.importAssets('selected-scene'); await native.api.readArtifact('text-id'); await native.api.exportTextAsset('text-id');
assert.deepEqual(plain(native.calls), [{ command: 'import_assets', args: { sceneId: 'selected-scene' } }, { command: 'read_artifact', args: { assetId: 'text-id' } }, { command: 'export_text_asset', args: { assetId: 'text-id' } }]);
checks.push('Native bridge pins import to the explicit scene and only exports registered asset IDs');
for (const check of checks) console.log(`PASS ${check}`);
console.log(`${checks.length} text attachment checks passed`);
