// SPDX-License-Identifier: MPL-2.0
// All GitHub responses and payloads are synthetic; no network or real keys.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { platforms, releasePlan, updateUrl, fingerprint, legacyUpdateUrl } from './release-contract.mjs';
import { legacyRepairManifest, validateLegacyMigrationAdvance, publishLegacyRepair } from './legacy-feed-migration.mjs';
const repository = 'example/project', version = '1.0.0', commit = 'a'.repeat(40);
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
function manifestFor(version, label, legacy = false) {
  return { version, notes: 'fixture', platforms: Object.fromEntries(platforms.map(platform => [platform, { url: (legacy ? legacyUpdateUrl : updateUrl)(version, platform), signature: Buffer.from(`untrusted comment: ${label}\nfixture\ntrusted comment: version:${version}\nfixture\n`).toString('base64') }])) };
}
function fixture(run) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'mewu-legacy-repair-'));
  try {
    const plan = releasePlan(version), manifest = manifestFor(version, 'new'), legacy = legacyRepairManifest(manifest);
    const names = [...Object.values(plan).flatMap(p => p.files), plan['darwin-aarch64'].updater, 'latest.json'];
    for (const name of names) fs.writeFileSync(path.join(directory, name), name === 'latest.json' ? JSON.stringify(manifest) : `new fixture ${name}`);
    const artifacts = names.map(name => ({ ...fingerprint(path.join(directory, name)), path: path.join(directory, name) }));
    const feedDir = path.join(directory, 'legacy'); fs.mkdirSync(feedDir);
    const feedFiles = Object.fromEntries(['preview.json', 'latest.json'].map(name => [name, path.join(feedDir, name)]));
    for (const file of Object.values(feedFiles)) fs.writeFileSync(file, JSON.stringify(legacy, null, 2) + '\n');
    let nextId = 500;
    const metadata = (name, raw, id) => ({ id: id ?? nextId++, name, size: Buffer.byteLength(raw), digest: `sha256:${hash(raw)}` });
    const formal = { id: 40, tag_name: 'v1.0.0', draft: false, prerelease: false, assets: artifacts.map(row => ({ id: nextId++, name: row.name, size: row.bytes, digest: `sha256:${row.sha256}` })) };
    const old = manifestFor(version, 'old', true), oldRaw = Buffer.from(JSON.stringify(old));
    const rawById = new Map(), channel = { id: 50, tag_name: 'update-channel', draft: false, prerelease: true, assets: [] };
    for (const name of ['preview.json', 'latest.json', plan['darwin-aarch64'].updater]) {
      const raw = name.endsWith('.json') ? oldRaw : Buffer.from('old updater fixture'), row = metadata(name, raw);
      channel.assets.push(row); rawById.set(row.id, raw);
    }
    const previous = row => ({ id: row.id, bytes: row.size, sha256: row.digest.slice(7) });
    const repairAuthorization = { schemaVersion: 1, version, sourceCommit: commit, channelReleaseId: channel.id, targetManifestSha256: artifacts.find(a => a.name === 'latest.json').sha256, targetUpdaterSha256: artifacts.find(a => a.name.endsWith('.tar.gz')).sha256, previousFeeds: Object.fromEntries(channel.assets.filter(a => a.name.endsWith('.json')).map(row => [row.name, previous(row)])), previousUpdater: { ...previous(channel.assets.at(-1)), name: plan['darwin-aarch64'].updater } };
    const calls = [], writes = [];
    let reads = 0, hook;
    const gh = (...args) => {
      calls.push(args);
      if (args[0] === 'api') {
        if (args[1].endsWith('/releases/tags/v1.0.0')) return JSON.stringify(formal);
        if (args[1].endsWith('/git/ref/tags/v1.0.0')) return JSON.stringify({ object: { type: 'commit', sha: commit } });
        if (args[1].endsWith('/releases/tags/update-channel')) { hook?.(++reads); return JSON.stringify(channel); }
        const match = /\/releases\/assets\/(\d+)$/.exec(args[1]);
        if (match) { assert.ok(args.includes('Accept: application/octet-stream')); return rawById.get(Number(match[1])).toString('utf8'); }
        throw Error(`Unexpected fixture endpoint ${args[1]}`);
      }
      assert.deepEqual(args.slice(0, 3), ['release', 'upload', 'update-channel']); assert.ok(args.includes('--clobber'));
      const file = args.at(-1), name = path.basename(file), raw = fs.readFileSync(file), row = metadata(name, raw);
      const index = channel.assets.findIndex(a => a.name === name); assert.ok(index >= 0, 'Never create unrelated assets');
      channel.assets[index] = row; rawById.set(row.id, raw); writes.push(name); return '';
    };
    const invoke = options => publishLegacyRepair(gh, repository, { manifest, commit, artifacts, feedFiles, repairAuthorization, verifyLocal: () => {}, ...options });
    run({ invoke, gh, manifest, legacy, old, channel, formal, artifacts, feedFiles, repairAuthorization, calls, writes, rawById, setHook: callback => { hook = callback; } });
  } finally {
    const resolved = fs.realpathSync(directory);
    assert.equal(path.dirname(resolved), fs.realpathSync(os.tmpdir())); assert.ok(path.basename(resolved).startsWith('mewu-legacy-repair-'));
    fs.rmSync(resolved, { recursive: true });
  }
}
test('old native Mac receives its exact channel path with original signatures and all other fields unchanged', () => {
  const manifest = manifestFor(version, 'new'), legacy = legacyRepairManifest(manifest);
  assert.notEqual(legacy.platforms['darwin-aarch64'].url, manifest.platforms['darwin-aarch64'].url);
  const restored = structuredClone(legacy); restored.platforms['darwin-aarch64'].url = manifest.platforms['darwin-aarch64'].url;
  assert.deepEqual(restored, manifest);
  assert.throws(() => legacyRepairManifest(manifestFor('1.0.1', 'new')), /fixed to stable 1.0.0/);
});
test('ordinary legacy validator still refuses same-version replacement and downgrade', () => {
  const old = manifestFor(version, 'old', true), next = manifestFor(version, 'new', true);
  assert.throws(() => validateLegacyMigrationAdvance(old, next), /Same version/);
  assert.throws(() => validateLegacyMigrationAdvance(manifestFor('1.0.1', 'old', true), next), /downgrade/);
  validateLegacyMigrationAdvance(manifestFor('1.0.0-preview.4', 'old', true), next);
});
test('exact-authorized same-version repair changes only the three old compatibility assets and is idempotent', () => fixture(({ invoke, writes, formal }) => {
  const before = structuredClone(formal), result = invoke();
  assert.deepEqual(writes, ['MewuAI-Remake-Update-1.0.0-mac-arm64.app.tar.gz', 'preview.json', 'latest.json']);
  assert.deepEqual(formal, before); assert.equal(result.formalReleaseModified, false); assert.equal(result.sameVersionInstalledStableAutoUpdate, false);
  const second = invoke(); assert.deepEqual(second.legacyFeedsUpdated, []); assert.equal(writes.length, 3);
}));
test('same-version repair without explicit hash-bound authorization performs no writes', () => fixture(({ invoke, writes }) => {
  assert.throws(() => invoke({ repairAuthorization: undefined }), /exact repair authorization/); assert.equal(writes.length, 0);
}));
for (const field of ['channelReleaseId', 'sourceCommit', 'targetManifestSha256', 'targetUpdaterSha256', 'previousFeedId', 'previousFeedHash', 'previousUpdaterId', 'previousUpdaterHash']) test(`repair rejects wrong ${field} before any write`, () => fixture(({ invoke, repairAuthorization, writes }) => {
  const wrong = structuredClone(repairAuthorization);
  if (field === 'channelReleaseId') wrong.channelReleaseId++;
  else if (field === 'sourceCommit') wrong.sourceCommit = 'b'.repeat(40);
  else if (field === 'previousFeedId') wrong.previousFeeds['latest.json'].id++;
  else if (field === 'previousFeedHash') wrong.previousFeeds['preview.json'].sha256 = 'b'.repeat(64);
  else if (field === 'previousUpdaterId') wrong.previousUpdater.id++;
  else if (field === 'previousUpdaterHash') wrong.previousUpdater.sha256 = 'b'.repeat(64);
  else wrong[field] = 'b'.repeat(64);
  assert.throws(() => invoke({ repairAuthorization: wrong })); assert.equal(writes.length, 0);
}));
test('a newer legacy feed is never overwritten even when its hash is explicitly supplied', () => fixture(({ invoke, channel, rawById, repairAuthorization, writes }) => {
  const asset = channel.assets[0], raw = Buffer.from(JSON.stringify(manifestFor('1.0.1', 'newer', true)));
  asset.size = raw.length; asset.digest = `sha256:${hash(raw)}`; rawById.set(asset.id, raw);
  repairAuthorization.previousFeeds[asset.name] = { id: asset.id, bytes: raw.length, sha256: hash(raw) };
  assert.throws(() => invoke(), /downgrade/); assert.equal(writes.length, 0);
}));
test('changed feed bytes and changed formal source assets fail closed before compatibility writes', () => {
  fixture(({ invoke, channel, rawById, writes }) => { rawById.set(channel.assets[0].id, Buffer.from('{}')); assert.throws(() => invoke(), /bytes differ/); assert.equal(writes.length, 0); });
  fixture(({ invoke, formal, writes }) => { formal.assets[0].digest = `sha256:${'b'.repeat(64)}`; assert.throws(() => invoke(), /Formal migration asset differs/); assert.equal(writes.length, 0); });
});
test('concurrent legacy advancement detected before mirror replacement performs no writes', () => fixture(({ invoke, channel, setHook, writes }) => {
  setHook(read => { if (read === 2) channel.assets[0].id++; });
  assert.throws(() => invoke()); assert.equal(writes.length, 0);
}));
test('prepared feed mutation and failed local acceptance never upload', () => {
  fixture(({ invoke, feedFiles, writes }) => { fs.writeFileSync(feedFiles['latest.json'], '{}'); assert.throws(() => invoke()); assert.equal(writes.length, 0); });
  fixture(({ invoke, calls, writes }) => { assert.throws(() => invoke({ verifyLocal: () => { throw Error('local'); } }), /local/); assert.equal(calls.length, 0); assert.equal(writes.length, 0); });
});
