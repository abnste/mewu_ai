// SPDX-License-Identifier: MPL-2.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { platforms, requiredChecks, releasePlan, updateUrl, fingerprint, verifyRelease, verifySignatureVersion, releaseVersion, compareReleaseVersions, validateChannelAdvance } from './release-contract.mjs';
const version = '1.0.0-preview.3', commit = 'a'.repeat(40);
const signature = Buffer.from(`untrusted comment: fixture\nfixture\ntrusted comment: timestamp:1\tversion:${version}\nfixture\n`).toString('base64');
function fixture(callback) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'mewu-release-gate-'));
  try {
    const plan = releasePlan(version), manifest = { version, platforms: {} }, acceptance = { sourceCommit: commit, version, platforms: {} };
    fs.mkdirSync(path.join(directory, '.metadata'));
    for (const platform of platforms) {
      const names = [...new Set([...plan[platform].files, plan[platform].updater])];
      for (const name of names) fs.writeFileSync(path.join(directory, name), `artifact ${platform} ${name}`);
      manifest.platforms[platform] = { signature, url: updateUrl(version, platform) };
      acceptance.platforms[platform] = { schemaVersion: 1, platform, version, sourceCommit: commit, result: 'passed', nativeGuiVerified: true, installerVerified: true, nativeTests: { passed: 10, failed: 0 }, frontendTests: { passed: 10, failed: 0 }, checks: Object.fromEntries(requiredChecks.map(key => [key, 'passed'])), artifacts: names.map(name => fingerprint(path.join(directory, name))) };
    }
    fs.writeFileSync(path.join(directory, '.metadata', 'update.json'), JSON.stringify(manifest));
    fs.writeFileSync(path.join(directory, '.metadata', 'acceptance.json'), JSON.stringify(acceptance));
    callback({ directory, manifest, acceptance, plan });
  } finally { fs.rmSync(directory, { recursive: true }); }
}
test('three native receipts bind installer and update bytes; mac update archive remains in channel', () => fixture(({ directory, manifest, acceptance }) => {
  const result = verifyRelease(directory, manifest, acceptance, commit);
  assert.equal(result.assets.length, 4);
  assert.deepEqual(result.channelAssets, [`MewuAI-Remake-Update-${version}-mac-arm64.app.tar.gz`]);
  assert.ok(result.assets.every(name => !name.endsWith('.sig')));
}));
for (const platform of platforms) {
  test(`${platform}: missing receipt refuses publication`, () => fixture(({ directory, manifest, acceptance }) => {
    delete acceptance.platforms[platform]; assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
  }));
  for (const field of ['nativeGuiVerified', 'installerVerified']) test(`${platform}: regression success cannot replace ${field}`, () => fixture(({ directory, manifest, acceptance }) => {
    acceptance.platforms[platform][field] = false; assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
  }));
  for (const check of requiredChecks) test(`${platform}: ${check} pending refuses publication`, () => fixture(({ directory, manifest, acceptance }) => {
    acceptance.platforms[platform].checks[check] = 'pending'; assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
  }));
  test(`${platform}: different source/version and failed suite refuse`, () => fixture(({ directory, manifest, acceptance }) => {
    const original = structuredClone(acceptance.platforms[platform]);
    for (const mutation of [r => r.sourceCommit = 'b'.repeat(40), r => r.version = '1.0.0-preview.2', r => r.nativeTests.failed = 1, r => r.frontendTests.passed = 0, r => r.artifacts[0].bytes += 1]) {
      acceptance.platforms[platform] = structuredClone(original); mutation(acceptance.platforms[platform]);
      assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
    }
  }));
  test(`${platform}: changed artifact refuses`, () => fixture(({ directory, manifest, acceptance, plan }) => {
    fs.appendFileSync(path.join(directory, plan[platform].updater), 'tampered');
    assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
  }));
  test(`${platform}: cross-platform and alternate-host update URL refuse`, () => fixture(({ directory, manifest, acceptance }) => {
    const value = manifest.platforms[platform];
    for (const url of [updateUrl(version, platforms.find(p => p !== platform)), value.url.replace('github.com', 'example.com'), `${value.url}?download=1`, value.url.replace(version, '1.0.0-preview.2')]) {
      value.url = url; assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
    }
  }));
}
test('missing third platform update refuses', () => fixture(({ directory, manifest, acceptance }) => {
  delete manifest.platforms['linux-x86_64']; assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
}));
test('signature version is required exactly once', () => {
  verifySignatureVersion(signature, version);
  for (const text of ['', 'trusted comment: version:1.0.0-preview.2', `trusted comment: version:${version}\tversion:${version}`, `trusted comment: timestamp:1`]) assert.throws(() => verifySignatureVersion(Buffer.from(text).toString('base64'), version));
});
for (const name of ['private-key.pem', 'debug.log', 'agent.md', 'extra.sig']) test(`unallowlisted ${name} refuses`, () => fixture(({ directory, manifest, acceptance }) => {
  fs.writeFileSync(path.join(directory, name), 'fixture'); assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
}));
test('unexpected metadata refuses', () => fixture(({ directory, manifest, acceptance }) => {
  fs.writeFileSync(path.join(directory, '.metadata', 'extra.json'), '{}'); assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
}));
test('unchanged filenames but replaced updater bytes cannot reuse old receipt', () => fixture(({ directory, manifest, acceptance, plan }) => {
  const target = path.join(directory, plan['darwin-aarch64'].updater), original = fs.readFileSync(target);
  const changed = Buffer.from(original); changed[0] ^= 1; fs.writeFileSync(target, changed);
  assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
}));
test('SemVer restricted grammar rejects leading zeros, build suffixes and oversized input', () => {
  for (const value of ['1.01.0', '1.0.01', '1.0.0-preview.03', '1.0.0+private', '1.0.0-preview.' + '1'.repeat(70)]) assert.throws(() => releaseVersion(value));
});
test('channel precedence handles numeric prereleases and release stability', () => {
  for (const [low, high] of [['1.0.0-preview.2', '1.0.0-preview.10'], ['1.0.0-rc.9', '1.0.0'], ['1.9.0', '1.10.0'], ['1.0.0-alpha.2', '1.0.0-beta.1'], ['1.0.0', '1.0.1-alpha.1']]) {
    assert.equal(compareReleaseVersions(low, high), -1); assert.equal(compareReleaseVersions(high, low), 1);
  }
  assert.equal(compareReleaseVersions(version, version), 0);
});
test('update channel cannot downgrade or change same-version bytes', () => fixture(({ manifest }) => {
  validateChannelAdvance(manifest, structuredClone(manifest));
  assert.throws(() => validateChannelAdvance({ version: '1.0.0' }, manifest));
  const changed = structuredClone(manifest); changed.platforms['darwin-aarch64'].signature += 'A';
  assert.throws(() => validateChannelAdvance(manifest, changed));
  validateChannelAdvance({ version: '1.0.0-preview.2', platforms: {} }, manifest);
}));
