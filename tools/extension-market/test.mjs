// SPDX-License-Identifier: MPL-2.0
// These are Node transport/filesystem tests; actual Rust contract tests are separate.
import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { deflateRawSync } from 'node:zlib';
import { buildCatalog, packPlugin, publishPlugin, run } from './cli.mjs';
import { crc32, createZip, packagePath, readZip } from './zip.mjs';
import { bindEntryManifest, downloadReleaseAsset, getRelease, githubBytes, sha256, verifyEntry } from './github.mjs';
import { collectPullRequest } from './collect-pr.mjs';
import { invokeExtensionValidator } from './validator.mjs';
import { createHash } from 'node:crypto';

const manifest = { schemaVersion: 2, id: 'tests.example', version: '1.0.0', name: '示例', description: '传输测试', license: 'MPL-2.0', hostApi: 2,
  runtime: 'javascript', entry: 'index.js', permissions: [], tags: ['tools'], dependencies: {} };
const files = () => [{ name: 'mewu-plugin.json', bytes: Buffer.from(JSON.stringify(manifest)) }, { name: 'index.js', bytes: Buffer.from('export function activate(ctx) {}') }];
const packageBytes = createZip(files());
const entry = { id: manifest.id, name: manifest.name, description: manifest.description, tags: manifest.tags, repository: 'example/plugins',
  version: manifest.version, releaseTag: 'v1.0.0', assetName: 'tests.example-1.0.0.zip', sha256: sha256(packageBytes), size: packageBytes.length, hostApi: 2, platforms: ['any'] };
const catalog = plugins => Buffer.from(`${JSON.stringify({ schemaVersion: 2, plugins }, null, 2)}\n`);
// A test double only for exercising atomic/network orchestration, never a product parser.
const validate = async (mode, bytes) => {
  const value = JSON.parse(bytes.toString());
  if (mode === 'extension-catalog') {
    if (new Set(value.plugins.map(item => item.id)).size !== value.plugins.length) throw new Error('duplicate id');
  }
  return value;
};
async function temporary(t) {
  const base = path.resolve(process.env.MEWU_PLUGIN_TEST_TMP ?? os.tmpdir());
  const directory = await mkdtemp(path.join(base, 'mewu-extension-test-'));
  t.after(async () => { assert.equal(path.dirname(directory), base); await rm(directory, { recursive: true, force: true }); });
  return directory;
}
async function fixture(t, items = [entry]) {
  const directory = await temporary(t), entries = path.join(directory, 'entries'), out = path.join(directory, 'mewu-catalog.json');
  await mkdir(entries);
  for (const item of items) await writeFile(path.join(entries, `${item.id}.json`), JSON.stringify(item));
  return { directory, input: entries, out };
}
function releaseFor(value = entry) {
  return { id: 20, tag_name: value.releaseTag, draft: false, assets: [{ id: 30, name: value.assetName, state: 'uploaded', size: value.size,
    digest: `sha256:${value.sha256}`, url: `https://api.github.com/repos/${value.repository}/releases/assets/30`,
    browser_download_url: `https://github.com/${value.repository}/releases/download/${value.releaseTag}/${value.assetName}` }] };
}
const response = value => new Response(JSON.stringify(value), { status: 200 });

test('ZIP byte stream is reproducible across file order and contains only declared files', () => {
  assert.deepEqual(createZip(files()), createZip(files().reverse()));
  assert.equal(crc32(Buffer.from('123456789')), 0xcbf43926);
  const unpacked = readZip(packageBytes);
  assert.equal(unpacked.size, 2); for (const file of files()) assert.deepEqual(unpacked.get(file.name), file.bytes);
});

test('ZIP rejects traversal, native files, secrets, caches and case collisions', () => {
  for (const name of ['../index.js', '/index.js', 'a/../index.js', 'a\\index.js', 'a//index.js', 'a.js/', 'C:/index.js', '.env', '.env.json',
    '.git/config.js', '.private/index.js', 'node_modules/a.js', 'AGENTS.md', 'plugin.exe', 'plugin.wasm', 'assets/icon.svg', 'NUL.js']) {
    assert.throws(() => packagePath(name), Error, name);
  }
  assert.throws(() => createZip([...files(), { name: 'INDEX.js', bytes: Buffer.from('x') }]), /冲突/);
  assert.throws(() => createZip([{ name: 'mewu-plugin.json', bytes: Buffer.from('\ufeff{}') }]), /BOM/);
  assert.throws(() => createZip([{ name: 'mewu-plugin.json', bytes: Buffer.from([255]) }]), /encoded data/);
});

test('ZIP rejects CRC corruption, local/central mismatch, links, encryption and undeclared bytes', () => {
  const central = packageBytes.readUInt32LE(packageBytes.length - 6);
  const mutate = fn => { const bytes = Buffer.from(packageBytes); fn(bytes); return bytes; };
  assert.throws(() => readZip(mutate(bytes => { bytes[30 + bytes.readUInt16LE(26)] ^= 1; })), /CRC32/);
  assert.throws(() => readZip(mutate(bytes => { bytes.writeUInt16LE(1, 8); })), /不一致/);
  assert.throws(() => readZip(mutate(bytes => { bytes.writeUInt32LE((0o120777 * 65536) >>> 0, central + 38); })), /链接/);
  assert.throws(() => readZip(mutate(bytes => { bytes.writeUInt16LE(0x801, central + 8); })), /加密/);
  assert.throws(() => readZip(Buffer.concat([Buffer.from('prefix'), packageBytes])), /中央目录/);
  assert.throws(() => readZip(Buffer.concat([packageBytes, Buffer.from('suffix')])), /单卷/);
  assert.throws(() => readZip(createZip([{ name: 'index.js', bytes: Buffer.from('x') }])), /根目录/);
});

test('ZIP safely supports ordinary deflated ZIP members without invoking author scripts', () => {
  const bytes = Buffer.from(JSON.stringify(manifest)), name = Buffer.from('mewu-plugin.json'), compressed = deflateRawSync(bytes);
  const local = Buffer.alloc(30), central = Buffer.alloc(46), end = Buffer.alloc(22);
  local.writeUInt32LE(0x04034b50); local.writeUInt16LE(20, 4); local.writeUInt16LE(8, 8); local.writeUInt32LE(crc32(bytes), 14);
  local.writeUInt32LE(compressed.length, 18); local.writeUInt32LE(bytes.length, 22); local.writeUInt16LE(name.length, 26);
  central.writeUInt32LE(0x02014b50); central.writeUInt16LE(20, 6); central.writeUInt16LE(8, 10); central.writeUInt32LE(crc32(bytes), 16);
  central.writeUInt32LE(compressed.length, 20); central.writeUInt32LE(bytes.length, 24); central.writeUInt16LE(name.length, 28);
  const body = Buffer.concat([local, name, compressed]), directory = Buffer.concat([central, name]);
  end.writeUInt32LE(0x06054b50); end.writeUInt16LE(1, 8); end.writeUInt16LE(1, 10); end.writeUInt32LE(directory.length, 12); end.writeUInt32LE(body.length, 16);
  assert.deepEqual(readZip(Buffer.concat([body, directory, end])).get('mewu-plugin.json'), bytes);
});

test('pack excludes undeclared local data and makes deterministic packages', async t => {
  const directory = await temporary(t), input = path.join(directory, 'plugin'), out = path.join(directory, 'one.zip'); await mkdir(input);
  for (const file of files()) await writeFile(path.join(input, file.name), file.bytes);
  await writeFile(path.join(input, '.env'), 'LOCAL TEST PRIVATE'); await writeFile(path.join(input, 'secret.json'), '{}');
  const result = await packPlugin({ input, out, files: [] }, validate);
  assert.equal(result.sha256, sha256(packageBytes)); assert.deepEqual(await readFile(out), packageBytes);
  await assert.rejects(packPlugin({ input, out, files: ['.env'] }, validate), /私有文件/);
  await assert.rejects(packPlugin({ input, out, files: ['index.js'] }, validate), /重复/);
  assert.deepEqual(await readFile(out), packageBytes);
});

test('pack rejects linked modules or parent directories before reading data', async t => {
  const directory = await temporary(t), input = path.join(directory, 'plugin'), outside = path.join(directory, 'outside'); await mkdir(input); await mkdir(outside);
  for (const file of files()) await writeFile(path.join(input, file.name), file.bytes);
  await writeFile(path.join(outside, 'helper.js'), 'SECRET OUTSIDE TEST');
  try { await symlink(outside, path.join(input, 'linked'), process.platform === 'win32' ? 'junction' : 'dir'); }
  catch (error) { if (['EPERM', 'EACCES'].includes(error.code)) { t.diagnostic('OS disallowed creating symlink fixture; directory link case not executed'); return; } throw error; }
  await assert.rejects(packPlugin({ input, out: path.join(directory, 'pkg.zip'), files: ['linked/helper.js'] }, validate), /链接/);
});

test('catalog build uses native result, deterministic order, real verification by default and atomic replacement', async t => {
  const other = { ...entry, id: 'another.plugin' }, options = await fixture(t, [entry, other]), verified = [];
  const result = await buildCatalog(options, validate, async item => { verified.push(item.id); return { id: item.id }; });
  assert.equal(result.releasesVerified, true); assert.deepEqual(verified, ['another.plugin', 'tests.example']);
  const expected = catalog([other, entry]); assert.deepEqual(await readFile(options.out), expected);
  assert.equal((await buildCatalog({ ...options, check: true }, validate, async () => ({}))).checked, true);
  assert.deepEqual(await readFile(options.out), expected);
});

test('icon feed keeps older strict hosts on an unchanged legacy catalog, including check mode', async t => {
  const icon = { paths: ['M4 4h16v16H4z'], color: '#3478f6', filled: false };
  const options = await fixture(t, [{ ...entry, icon }]), legacyOut = path.join(options.directory, 'legacy.json');
  await writeFile(legacyOut, catalog([entry]));
  const before = await readFile(legacyOut), verified = [];
  await buildCatalog({ ...options, legacyOut, previous: legacyOut }, validate, async value => { verified.push(value); return {}; });
  assert.deepEqual(verified.map(value => value.icon), [icon]);
  assert.deepEqual(JSON.parse(await readFile(options.out)).plugins[0].icon, icon);
  assert.deepEqual(await readFile(legacyOut), before);
  await buildCatalog({ ...options, legacyOut, check: true, offline: true }, validate);
  await writeFile(legacyOut, '{}');
  await assert.rejects(buildCatalog({ ...options, legacyOut, check: true, offline: true }, validate), /兼容目录/);
  for (const filename of [options.out, path.join(options.input, 'legacy.json')]) {
    await assert.rejects(buildCatalog({ ...options, legacyOut: filename, offline: true }, validate), /兼容目录输出/);
  }
});

test('market branding may decorate old packages, but cannot replace a package author icon', () => {
  const icon = { paths: ['M0 0L24 24'], color: '#ff3344', filled: false };
  bindEntryManifest({ ...entry, icon }, manifest);
  bindEntryManifest({ ...entry, icon: { filled: false, color: '#ff3344', paths: icon.paths } }, { ...manifest, icon });
  assert.throws(() => bindEntryManifest({ ...entry, icon: { ...icon, color: '#000000' } }, { ...manifest, icon }), /图标/);
});

test('catalog duplicate identity, filename mismatch, invalid inputs and remote failures preserve old bytes', async t => {
  const options = await fixture(t); await writeFile(options.out, catalog([])); const original = await readFile(options.out);
  await assert.rejects(buildCatalog(options, validate, async () => { throw new Error('remote hash mismatch'); }), /hash mismatch/);
  assert.deepEqual(await readFile(options.out), original);
  await writeFile(path.join(options.input, 'extra.txt'), 'x'); await assert.rejects(buildCatalog(options, validate), /直属/); await rm(path.join(options.input, 'extra.txt'));
  await writeFile(path.join(options.input, 'other.json'), JSON.stringify(entry)); await assert.rejects(buildCatalog(options, validate), /文件名/); await rm(path.join(options.input, 'other.json'));
  await assert.rejects(buildCatalog({ ...options, out: path.join(options.input, 'bad.json') }, validate), /entries/);
  assert.deepEqual(await readFile(options.out), original);
  assert.deepEqual((await readdir(options.directory)).sort(), ['entries', 'mewu-catalog.json']);
});

test('existing catalog identity cannot silently switch its GitHub repository', async t => {
  const options = await fixture(t, [{ ...entry, repository: 'attacker/other' }]); await writeFile(options.out, catalog([entry]));
  await assert.rejects(buildCatalog({ ...options, offline: true }, validate), /其他 GitHub 来源/);
  assert.deepEqual(await readFile(options.out), catalog([entry]));
});

test('existing version bytes cannot change under an unchanged plugin version', async t => {
  const options = await fixture(t, [{ ...entry, sha256: '0'.repeat(64) }]); await writeFile(options.out, catalog([entry]));
  await assert.rejects(buildCatalog({ ...options, offline: true }, validate), /同一版本/);
  assert.deepEqual(await readFile(options.out), catalog([entry]));
});

test('empty catalog is valid, no fake seed entries are invented, offline mode is explicit', async t => {
  const options = await fixture(t, []); let requests = 0;
  const result = await buildCatalog(options, validate, async () => { requests++; });
  assert.equal(result.count, 0); assert.equal(requests, 0); assert.deepEqual(await readFile(options.out), catalog([]));
  assert.equal((await buildCatalog({ ...options, offline: true }, validate)).releasesVerified, false);
});

test('GitHub verification binds exact release asset, bytes, hash and original manifest', async () => {
  const calls = [], fetchImpl = async (url, options) => { calls.push([String(url), options]);
    return String(url).includes('/releases/tags/') ? response(releaseFor()) : new Response(packageBytes, { status: 200 }); };
  const result = await verifyEntry(entry, validate, { fetchImpl });
  assert.equal(result.sha256, entry.sha256); assert.equal(result.assetId, 30); assert.equal(calls.length, 2);
  assert.equal(calls[1][1].headers.Accept, 'application/octet-stream');
  await assert.rejects(verifyEntry({ ...entry, sha256: '0'.repeat(64) }, validate, { fetchImpl }), /SHA256/);
  await assert.rejects(verifyEntry({ ...entry, name: 'pretend' }, validate, { fetchImpl }), /name/);
  assert.throws(() => bindEntryManifest({ ...entry, tags: ['fake'] }, manifest), /标签/);
});

test('catalog platform scope may narrow a package but cannot widen its explicit targets', () => {
  assert.doesNotThrow(() => bindEntryManifest({ ...entry, platforms: ['windows-x86_64'] }, manifest));
  assert.doesNotThrow(() => bindEntryManifest(entry, { ...manifest, platforms: ['any'] }));
  const restricted = { ...manifest, platforms: ['darwin-aarch64', 'linux-x86_64'] };
  assert.doesNotThrow(() => bindEntryManifest({ ...entry, platforms: ['darwin-aarch64'] }, restricted));
  assert.throws(() => bindEntryManifest(entry, restricted), /平台范围/);
  assert.throws(() => bindEntryManifest({ ...entry, platforms: ['windows-x86_64'] }, restricted), /平台范围/);
});

test('GitHub rejects drafts, asset substitution, duplicate asset names and truncated downloads', async () => {
  await assert.rejects(getRelease(entry.repository, entry.releaseTag, { fetchImpl: async () => response({ ...releaseFor(), draft: true }) }), /发行版本/);
  const release = releaseFor(); release.assets[0].url = 'https://api.github.com/repos/attacker/other/releases/assets/30';
  await assert.rejects(downloadReleaseAsset(entry.repository, release, entry.assetName, entry), /来源/);
  const duplicate = releaseFor(); duplicate.assets.push(duplicate.assets[0]); await assert.rejects(downloadReleaseAsset(entry.repository, duplicate, entry.assetName), /唯一/);
  await assert.rejects(downloadReleaseAsset(entry.repository, releaseFor(), entry.assetName, entry, { fetchImpl: async () => new Response(packageBytes.subarray(1)) }), /SHA256/);
});

test('GitHub download permits only HTTPS GitHub asset redirects and never forwards credentials', async t => {
  const previous = process.env.GH_TOKEN; process.env.GH_TOKEN = 'test-token-not-real';
  t.after(() => { if (previous === undefined) delete process.env.GH_TOKEN; else process.env.GH_TOKEN = previous; });
  const seen = [];
  const fetched = await githubBytes('https://api.github.com/repos/example/plugins/releases/assets/30', { binary: true,
    fetchImpl: async (url, options) => { seen.push([String(url), options.headers]); return seen.length === 1
      ? new Response(null, { status: 302, headers: { location: 'https://release-assets.githubusercontent.com/example?download=test' } }) : new Response('bytes'); } });
  assert.equal(fetched.toString(), 'bytes'); assert.equal(seen[0][1].Authorization, 'Bearer test-token-not-real'); assert.equal(seen[1][1].Authorization, undefined);
  for (const location of ['http://release-assets.githubusercontent.com/test', 'https://127.0.0.1/test', 'https://github.com.attacker.test/x', 'https://user:pass@objects.githubusercontent.com/test']) {
    await assert.rejects(githubBytes('https://api.github.com/test', { binary: true, fetchImpl: async () => new Response(null, { status: 302, headers: { location } }) }), /允许来源/);
  }
  await assert.rejects(githubBytes('https://api.github.com/test', { fetchImpl: async () => new Response('large', { headers: { 'content-length': '999999999' } }) }), /体积/);
});

test('publish never overwrites conflicting asset and validates all local identity before upload', async t => {
  const directory = await temporary(t), input = path.join(directory, entry.assetName), entryOut = path.join(directory, `${entry.id}.json`); await writeFile(input, packageBytes);
  let uploads = 0;
  const network = { getRelease: async () => releaseFor({ ...entry, sha256: '0'.repeat(64) }), upload: async () => { uploads++; } };
  await assert.rejects(publishPlugin({ input, entryOut, repository: entry.repository, releaseTag: entry.releaseTag }, validate, network), /禁止覆盖/); assert.equal(uploads, 0);
  await assert.rejects(publishPlugin({ input, entryOut: path.join(directory, 'wrong.json'), repository: entry.repository, releaseTag: entry.releaseTag }, validate, network), /id.json/); assert.equal(uploads, 0);
  await writeFile(entryOut, JSON.stringify({ ...entry, repository: 'prior/owner' }));
  await assert.rejects(publishPlugin({ input, entryOut, repository: entry.repository, releaseTag: entry.releaseTag }, validate, network), /GitHub 来源/); assert.equal(uploads, 0);
});

test('publish uploads only once, verifies actual downloaded ZIP, then writes entry', async t => {
  const directory = await temporary(t), input = path.join(directory, entry.assetName), entryOut = path.join(directory, `${entry.id}.json`); await writeFile(input, packageBytes);
  let uploads = 0;
  const network = { getRelease: async () => uploads ? releaseFor() : { ...releaseFor(), assets: [] }, upload: async () => { uploads++; },
    downloadReleaseAsset: async (_repository, _release, _name, expected) => { assert.equal(expected.sha256, entry.sha256); return { bytes: packageBytes }; } };
  const result = await publishPlugin({ input, entryOut, repository: entry.repository, releaseTag: entry.releaseTag }, validate, network);
  assert.equal(result.verified, true); assert.equal(uploads, 1); assert.deepEqual(JSON.parse(await readFile(entryOut, 'utf8')), entry);
  assert.equal((await publishPlugin({ input, entryOut, repository: entry.repository, releaseTag: entry.releaseTag }, validate, network)).uploaded, false); assert.equal(uploads, 1);
});

test('native wrapper rejects stale contracts, malformed output, process errors and timeout', async t => {
  const directory = await temporary(t), script = path.join(directory, 'fixture.mjs');
  await writeFile(script, 'process.stdin.resume();process.stdin.on("end",()=>console.log(JSON.stringify({protocolVersion:1,extensionContractVersion:2,extensionContractSha256:"expected",ok:true,value:{id:"ok"}})));');
  assert.deepEqual(await invokeExtensionValidator(process.execPath, script, Buffer.from('{}'), 'expected'), { id: 'ok' });
  await assert.rejects(invokeExtensionValidator(process.execPath, script, Buffer.from('{}'), 'stale'), /契约/);
  await writeFile(script, 'console.log("not-json");'); await assert.rejects(invokeExtensionValidator(process.execPath, script, Buffer.from('{}'), 'expected'), /有效 JSON/);
  await writeFile(script, 'process.stdin.resume();setInterval(()=>{},1000);'); await assert.rejects(invokeExtensionValidator(process.execPath, script, Buffer.from('{}'), 'expected', 100), /超时/);
});

test('PR collector reads fixed Git blobs as data and rejects author code or stale head', async t => {
  const directory = await temporary(t), raw = Buffer.from(JSON.stringify(entry)), blobSha = createHash('sha1').update(`blob ${raw.length}\0`).update(raw).digest('hex');
  const head = 'a'.repeat(40), event = { number: 1, repository: { full_name: 'market/catalog' }, pull_request: { head: { sha: head } } };
  const pr = { state: 'open', changed_files: 1, base: { repo: { full_name: 'market/catalog' } }, head: { sha: head, repo: { full_name: 'author/catalog' } } };
  let changed = [{ filename: `entries/${entry.id}.json`, status: 'added' }];
  const fetchImpl = async url => { const value = String(url); return response(value.endsWith('/pulls/1') ? pr : value.includes('/files?') ? changed
    : value.includes('/git/trees/') ? { truncated: false, tree: [{ path: `entries/${entry.id}.json`, type: 'blob', mode: '100644', sha: blobSha, size: raw.length }] }
      : { sha: blobSha, encoding: 'base64', content: raw.toString('base64') }); };
  const output = path.join(directory, 'candidate');
  assert.equal((await collectPullRequest({ event, repository: 'market/catalog', output }, { fetchImpl })).head, head);
  assert.deepEqual(await readFile(path.join(output, `${entry.id}.json`)), raw);
  changed = [{ filename: 'scripts/evil.js', status: 'added' }];
  await assert.rejects(collectPullRequest({ event, repository: 'market/catalog', output: path.join(directory, 'rejected') }, { fetchImpl }), /只能修改/);
  changed = [{ filename: `entries/${entry.id}.json`, status: 'added' }]; pr.head.sha = 'b'.repeat(40);
  await assert.rejects(collectPullRequest({ event, repository: 'market/catalog', output: path.join(directory, 'stale') }, { fetchImpl }), /PR 已改变/);
});

test('CLI usage errors do not invoke a validator or write files', async () => {
  let output = '', invocations = 0; const io = { stdout: { write: text => { output += text; } }, stderr: { write: text => { output += text; } } };
  assert.equal(await run(['--help'], io), 0); assert.match(output, /API 2/);
  assert.equal(await run(['build', '.', '--unknown', '--json'], io, { validate: async () => { invocations++; } }), 1); assert.equal(invocations, 0);
});
