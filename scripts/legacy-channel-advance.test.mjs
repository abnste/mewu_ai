// SPDX-License-Identifier: MPL-2.0
// Real local bytes and a simulated GitHub transport; no network or signing keys.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { repository, platforms, releasePlan, updateUrl, legacyUpdateUrl, fingerprint } from './release-contract.mjs';
import { legacyAdvanceManifest, publishLegacyAdvance } from './legacy-channel-advance.mjs';

const version = '1.0.1', commit = 'a'.repeat(40), tar = releasePlan(version)['darwin-aarch64'].updater;
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
function manifestFor(value, label, legacy = false) {
  return { version: value, notes: label, platforms: Object.fromEntries(platforms.map(platform => [platform, {
    url: (legacy ? legacyUpdateUrl : updateUrl)(value, platform),
    signature: Buffer.from(`untrusted comment: fixture\nfixture\ntrusted comment: version:${value}\nfixture\n`).toString('base64'),
  }])) };
}
function fixture(run) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'mewu-legacy-advance-'));
  try {
    const manifest = manifestFor(version, 'candidate'), legacy = legacyAdvanceManifest(manifest), plan = releasePlan(version);
    const names = [...Object.values(plan).flatMap(entry => entry.files), tar, 'latest.json'];
    for (const name of names) fs.writeFileSync(path.join(directory, name), name === 'latest.json' ? JSON.stringify(manifest) : `fixture bytes ${name}`);
    const artifacts = names.map(name => ({ ...fingerprint(path.join(directory, name)), path: path.join(directory, name) }));
    const feedRoot = path.join(directory, 'feeds'); fs.mkdirSync(feedRoot);
    const feedFiles = Object.fromEntries(['preview.json', 'latest.json'].map(name => [name, path.join(feedRoot, name)]));
    for (const file of Object.values(feedFiles)) fs.writeFileSync(file, JSON.stringify(legacy, null, 2) + '\n');
    let nextId = 500, runs = 0, reads = 0, hook, uploadFailure;
    const rawById = new Map(), calls = [], writes = [], downloads = [], progress = [];
    function metadata(name, raw, channel = false) {
      const row = { id: nextId++, name, size: raw.length, digest: `sha256:${hash(raw)}`,
        browser_download_url: `https://github.com/${repository}/releases/download/${channel ? 'update-channel' : `v${version}`}/${name}` };
      rawById.set(row.id, Buffer.from(raw)); return row;
    }
    const formal = { id: 40, tag_name: `v${version}`, draft: false, prerelease: false,
      assets: artifacts.map(file => metadata(file.name, fs.readFileSync(file.path))) };
    const latest = structuredClone(formal);
    const channel = { id: 50, tag_name: 'update-channel', draft: false, prerelease: true, assets: [] };
    const setChannelAsset = (name, raw) => {
      const row = metadata(name, raw, true), index = channel.assets.findIndex(asset => asset.name === name);
      if (index < 0) channel.assets.push(row); else channel.assets[index] = row;
      return row;
    };
    const oldRaw = Buffer.from(JSON.stringify(manifestFor('1.0.0', 'old', true)));
    for (const name of ['preview.json', 'latest.json']) setChannelAsset(name, oldRaw);
    const oldTar = setChannelAsset(releasePlan('1.0.0')['darwin-aarch64'].updater, Buffer.from('original Mac updater bytes'));
    const gh = (...args) => {
      calls.push(args);
      if (args[0] === 'api') {
        if (args[1].endsWith(`/releases/tags/v${version}`)) return JSON.stringify(formal);
        if (args[1].endsWith(`/git/ref/tags/v${version}`)) return JSON.stringify({ object: { type: 'commit', sha: commit } });
        if (args[1].endsWith('/releases/latest')) return JSON.stringify(latest);
        if (args[1].endsWith('/releases/tags/update-channel')) { hook?.(++reads); return JSON.stringify(channel); }
        const match = /\/releases\/assets\/(\d+)$/.exec(args[1]);
        if (match) return rawById.get(Number(match[1]));
        throw Error(`Unexpected synthetic read ${args[1]}`);
      }
      assert.deepEqual(args.slice(0, 3), ['release', 'upload', 'update-channel']);
      const file = args.at(-1), name = path.basename(file), clobber = args.includes('--clobber');
      assert.equal(clobber, name.endsWith('.json'), 'Never overwrite any versioned TAR');
      assert.ok(['preview.json', 'latest.json', tar].includes(name), 'No other asset mutation');
      writes.push({ name, clobber });
      if (uploadFailure?.name === name && uploadFailure.phase === 'before') throw Error('synthetic transport failure');
      if (uploadFailure?.name === name && uploadFailure.phase === 'after-delete') {
        channel.assets = channel.assets.filter(asset => asset.name !== name);
        throw Error('synthetic clobber interrupted after deletion');
      }
      setChannelAsset(name, fs.readFileSync(file));
      if (uploadFailure?.name === name && uploadFailure.phase === 'after') throw Error('synthetic response loss after server commit');
      return '';
    };
    const invoke = options => {
      const workDirectory = path.join(directory, `downloads-${++runs}`); fs.mkdirSync(workDirectory);
      return publishLegacyAdvance(gh, repository, { manifest, commit, artifacts, feedFiles, workDirectory,
        verifyLocal: () => {}, onProgress: event => progress.push(event),
        downloadAsset: (asset, destination) => { downloads.push(asset.name); fs.writeFileSync(destination, rawById.get(asset.id), { flag: 'wx' }); }, ...options });
    };
    run({ invoke, formal, latest, channel, oldTar, artifacts, feedFiles, manifest, legacy, rawById,
      calls, writes, downloads, progress, setChannelAsset,
      setHook: value => { hook = value; }, failUpload: value => { uploadFailure = value; } });
  } finally {
    const actual = fs.realpathSync(directory);
    assert.equal(path.dirname(actual), fs.realpathSync(os.tmpdir()));
    assert.ok(path.basename(actual).startsWith('mewu-legacy-advance-'));
    fs.rmSync(actual, { recursive: true });
  }
}

test('only newer stable versions derive legacy URLs without changing inline signatures', () => {
  const original = manifestFor(version, 'candidate'), legacy = legacyAdvanceManifest(original);
  assert.equal(legacy.platforms['darwin-aarch64'].url, legacyUpdateUrl(version, 'darwin-aarch64'));
  for (const platform of platforms) assert.equal(legacy.platforms[platform].signature, original.platforms[platform].signature);
  for (const value of ['1.0.0', '1.0.1-preview.1']) assert.throws(() => legacyAdvanceManifest(manifestFor(value, 'invalid')), /newer than 1.0.0/);
});
test('downloads six public formal assets before adding a new mirror and advancing both old feeds', () => fixture(context => {
  const { invoke, writes, formal, channel, oldTar, rawById } = context;
  const before = structuredClone(formal), oldBytes = Buffer.from(rawById.get(oldTar.id));
  const result = invoke();
  assert.deepEqual(writes, [{ name: tar, clobber: false }, { name: 'preview.json', clobber: true }, { name: 'latest.json', clobber: true }]);
  assert.equal(result.formalAssetBytesVerified, 6); assert.equal(result.downloads.length, 7);
  assert.deepEqual(formal, before); assert.ok(channel.assets.some(asset => asset.id === oldTar.id));
  assert.deepEqual(rawById.get(oldTar.id), oldBytes); assert.equal(result.unrelatedChannelAssetsPreserved, true);
  assert.deepEqual(invoke().legacyFeedsUpdated, []); assert.equal(writes.length, 3);
}));
test('actual public download tampering fails before any channel mutation', () => fixture(({ invoke, formal, rawById, writes }) => {
  rawById.set(formal.assets[0].id, Buffer.from('tampered body with unchanged metadata'));
  assert.throws(() => invoke(), /Downloaded published asset bytes differ/); assert.equal(writes.length, 0);
}));
for (const kind of ['draft', 'missing-asset', 'different-latest']) test(`formal ${kind} cannot advance a legacy channel`, () => fixture(({ invoke, formal, latest, writes }) => {
  if (kind === 'draft') formal.draft = true;
  if (kind === 'missing-asset') formal.assets.pop();
  if (kind === 'different-latest') latest.id++;
  assert.throws(() => invoke()); assert.equal(writes.length, 0);
}));
for (const value of ['1.0.1', '1.0.2']) test(`nonidentical legacy ${value} is never overwritten`, () => fixture(({ invoke, setChannelAsset, writes }) => {
  setChannelAsset('latest.json', Buffer.from(JSON.stringify(manifestFor(value, 'other', true))));
  assert.throws(() => invoke(), /Same-version|downgrade/); assert.equal(writes.length, 0);
}));
test('a conflicting versioned mirror is never overwritten', () => fixture(({ invoke, setChannelAsset, writes }) => {
  setChannelAsset(tar, Buffer.from('different published mirror'));
  assert.throws(() => invoke(), /never overwrite/); assert.equal(writes.length, 0);
}));
test('matching mirror metadata still requires its actual downloaded bytes', () => fixture(({ invoke, artifacts, setChannelAsset, rawById, writes }) => {
  const mirror = setChannelAsset(tar, fs.readFileSync(artifacts.find(file => file.name === tar).path));
  rawById.set(mirror.id, Buffer.from('tampered mirror download'));
  assert.throws(() => invoke(), /Downloaded published asset bytes differ/); assert.equal(writes.length, 0);
}));
for (const phase of ['before', 'after']) test(`unknown mirror upload outcome (${phase}) stops without a retry or feed writes`, () => fixture(({ invoke, failUpload, writes, progress }) => {
  failUpload({ name: tar, phase });
  assert.throws(() => invoke(), error => error.code === 'LEGACY_ADVANCE_UNKNOWN_UPLOAD_OUTCOME'
    && error.mutationMayHaveCompleted && error.automaticRetryAllowed === false);
  assert.equal(writes.length, 1); assert.ok(progress.some(event => event.phase === 'unknown-mutation-outcome'));
  failUpload(null); const result = invoke();
  assert.equal(result.formalReleaseModified, false);
  assert.equal(writes.filter(write => write.name === tar).length, phase === 'after' ? 1 : 2);
}));
test('a committed first feed with a lost response is inspectable and explicitly resumable', () => fixture(({ invoke, failUpload, writes }) => {
  failUpload({ name: 'preview.json', phase: 'after' });
  assert.throws(() => invoke(), /outcome is unknown/); assert.deepEqual(writes.map(write => write.name), [tar, 'preview.json']);
  failUpload(null); assert.deepEqual(invoke().legacyFeedsUpdated, ['latest.json']);
  assert.deepEqual(writes.map(write => write.name), [tar, 'preview.json', 'latest.json']);
}));
test('interrupted clobber with a missing feed remains stopped for explicit inspection', () => fixture(({ invoke, failUpload, writes, progress }) => {
  failUpload({ name: 'preview.json', phase: 'after-delete' });
  assert.throws(() => invoke(), /outcome is unknown/);
  const action = progress.findLast(event => event.phase === 'before-mutation').action;
  assert.ok(action.previousAsset.id > 0); assert.equal(action.channelReleaseId, 50);
  assert.match(action.targetSHA256, /^[a-f0-9]{64}$/);
  failUpload(null);
  assert.throws(() => invoke(), /Legacy feed missing/);
  assert.deepEqual(writes.map(write => write.name), [tar, 'preview.json']);
}));
test('a concurrent unrelated asset change stops before mutation', () => fixture(({ invoke, setHook, oldTar, writes }) => {
  setHook(read => { if (read === 2) oldTar.digest = `sha256:${'b'.repeat(64)}`; });
  assert.throws(() => invoke(), /Unrelated legacy assets changed/); assert.equal(writes.length, 0);
}));
test('invalid source acceptance and changed prepared feeds never upload', () => {
  fixture(({ invoke, calls, writes }) => { assert.throws(() => invoke({ verifyLocal: () => { throw Error('acceptance'); } }), /acceptance/); assert.equal(calls.length, 0); assert.equal(writes.length, 0); });
  fixture(({ invoke, feedFiles, writes }) => { fs.writeFileSync(feedFiles['preview.json'], '{}'); assert.throws(() => invoke()); assert.equal(writes.length, 0); });
});
